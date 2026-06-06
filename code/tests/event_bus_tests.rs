//! 事件总线测试。
//!
//! 这些测试覆盖基于 `session_id`、`agent_id` 的路由，以及模型输出事件的最小闭环。

use dshns_agent::app::agent_runner::{
    AgentRoundRequest, AgentRunner, AgentRunnerConfig, ModelGatewayError, ModelGatewayRequest,
    ModelGatewayResponse, ModelGatewayTrait,
};
use dshns_agent::app::workspace_session_service::{
    CreateSessionRequest, EnsureWorkspaceRequest, WorkspaceSessionService,
};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::event_bus::{EventBus, EventEnvelope, EventType};
use dshns_agent::infra::repository::EventLogRepository;
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

struct FakeEnvSource {
    values: HashMap<String, String>,
}

impl FakeEnvSource {
    fn new(values: impl IntoIterator<Item = (&'static str, &'static str)>) -> Self {
        Self {
            values: values
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }
}

impl EnvSource for FakeEnvSource {
    fn read(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }
}

#[derive(Clone)]
struct ScriptedModelGateway {
    responses: Rc<RefCell<Vec<ModelGatewayResponse>>>,
}

impl ScriptedModelGateway {
    fn new(responses: Vec<ModelGatewayResponse>) -> Self {
        Self {
            responses: Rc::new(RefCell::new(responses)),
        }
    }
}

impl ModelGatewayTrait for ScriptedModelGateway {
    fn complete(
        &self,
        _request: ModelGatewayRequest,
    ) -> Result<ModelGatewayResponse, ModelGatewayError> {
        let mut responses = self.responses.borrow_mut();
        Ok(responses.remove(0))
    }
}

#[test]
fn 应按会话和智能体路由事件并写入事件日志() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("event-routing-workspace");
    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("事件工作区".to_string()),
        })
        .expect("创建目录失败");
    let session_a = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id.clone(),
            first_prompt: "会话A".to_string(),
        })
        .expect("创建会话A失败");
    let session_b = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id,
            first_prompt: "会话B".to_string(),
        })
        .expect("创建会话B失败");
    let event_bus = EventBus::new(&database);

    event_bus
        .register_session(&session_a.session_id)
        .expect("注册会话 A 失败");
    event_bus
        .register_session(&session_b.session_id)
        .expect("注册会话 B 失败");

    let delivered_a = event_bus
        .publish(EventEnvelope::new(
            EventType::UserMessageReceived,
            &session_a.session_id,
            None,
            Some("ROUND-0001"),
            json!({"text":"A"}),
        ))
        .expect("投递会话 A 事件失败");
    let delivered_b = event_bus
        .publish(EventEnvelope::new(
            EventType::MetricsUpdated,
            &session_b.session_id,
            Some("AGT-B"),
            Some("ROUND-0002"),
            json!({"remaining_context": 100}),
        ))
        .expect("投递会话 B 事件失败");

    assert_eq!(delivered_a.session_id, session_a.session_id);
    assert_eq!(delivered_b.session_id, session_b.session_id);
    assert_eq!(
        event_bus
            .drain_session_events(&delivered_a.session_id)
            .expect("读取 A 事件失败")
            .len(),
        1
    );
    assert_eq!(
        event_bus
            .drain_session_events(&delivered_b.session_id)
            .expect("读取 B 事件失败")
            .len(),
        1
    );

    let repository = EventLogRepository::new(database.connection());
    let session_a_logs = repository
        .list_by_session(&delivered_a.session_id)
        .expect("查询事件日志失败");
    assert_eq!(session_a_logs.len(), 1);
    assert_eq!(session_a_logs[0].event_type, "user_message_received");
    assert_eq!(session_a_logs[0].status, "handled");
}

#[test]
fn 未找到目标会话时应标记丢弃而不广播() {
    let database = create_initialized_database();
    let event_bus = EventBus::new(&database);

    let dropped = event_bus
        .publish(EventEnvelope::new(
            EventType::ErrorRaised,
            "SES-MISSING",
            None,
            Some("ROUND-404"),
            json!({"message":"会话不存在"}),
        ))
        .expect("投递丢弃事件失败");

    assert_eq!(dropped.status, "dropped");
    assert!(
        event_bus
            .drain_session_events("SES-MISSING")
            .expect("读取缺失会话事件失败")
            .is_empty()
    );
}

#[test]
fn 单轮执行器应发出模型思考增量和最终输出事件() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("event-runner-workspace");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");
    let target_file = workspace_root.join("demo.txt");
    fs::write(&target_file, "事件总线测试内容").expect("写入测试文件失败");

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("事件目录".to_string()),
        })
        .expect("创建目录失败");
    let session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id.clone(),
            first_prompt: "初始消息".to_string(),
        })
        .expect("创建会话失败");

    let event_bus = EventBus::new(&database);
    event_bus
        .register_session(&session.session_id)
        .expect("注册会话失败");

    let gateway = ScriptedModelGateway::new(vec![
        ModelGatewayResponse::ToolCall {
            tool_name: "read_file".to_string(),
            arguments: json!({
                "path": target_file.to_string_lossy().to_string()
            }),
            tool_call_id: "call-event-1".to_string(),
            assistant_content: None,
        },
        ModelGatewayResponse::FinalText {
            content: "事件驱动链路完成。".to_string(),
        },
    ]);
    let runner = AgentRunner::new(
        &database,
        &config,
        gateway,
        AgentRunnerConfig::new(workspace_root, skill_root),
    )
    .with_event_bus(event_bus.clone());

    runner
        .run_round(AgentRoundRequest {
            session_id: session.session_id.clone(),
            agent_id: "AGT-0001".to_string(),
            user_input: "请读取文件并输出".to_string(),
            input_already_persisted: false,
            existing_round_id: None,
        })
        .expect("执行单轮流程失败");

    let events = event_bus
        .drain_session_events(&session.session_id)
        .expect("读取事件失败");
    let event_types = events
        .iter()
        .map(|event| event.event_type.clone())
        .collect::<Vec<_>>();

    assert!(event_types.contains(&"model_thinking_started".to_string()));
    assert!(event_types.contains(&"tool_started".to_string()));
    assert!(event_types.contains(&"tool_finished".to_string()));
    assert!(event_types.contains(&"assistant_output_delta".to_string()));
    assert!(event_types.contains(&"assistant_output_completed".to_string()));
}

fn create_initialized_database() -> SqliteDatabase {
    let database = SqliteDatabase::open(DatabaseTarget::InMemory).expect("打开内存数据库失败");
    database.initialize().expect("初始化数据库失败");
    database
}

fn create_temp_directory(prefix: &str) -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于纪元")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("dshns-{prefix}-{unique_suffix}"));
    fs::create_dir_all(&path).expect("创建临时目录失败");
    path
}
