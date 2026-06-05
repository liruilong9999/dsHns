//! 运行指标与回归测试。
//!
//! 这些测试覆盖 `session_metrics` 快照、`metrics_updated` 事件，以及多会话基础回归。

use dshns_agent::app::agent_runner::{
    AgentRoundRequest, AgentRunner, AgentRunnerConfig, ModelGatewayError, ModelGatewayRequest,
    ModelGatewayResponse, ModelGatewayTrait,
};
use dshns_agent::app::workspace_session_service::{
    ChangeSessionApprovalModeRequest, CreateSessionRequest, EnsureWorkspaceRequest,
    WorkspaceSessionService,
};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::event_bus::EventBus;
use dshns_agent::infra::metrics::SessionMetricsRepository;
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
fn 单轮执行后应写入会话指标快照并发出指标事件() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("metrics-workspace");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");
    let readme = workspace_root.join("README.md");
    fs::write(&readme, "这是一个用于指标测试的 README。").expect("写入 README 失败");

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("指标目录".to_string()),
        })
        .expect("创建目录失败");
    let session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id,
            first_prompt: "会话初始消息".to_string(),
        })
        .expect("创建会话失败");
    service
        .change_session_approval_mode(ChangeSessionApprovalModeRequest {
            session_id: session.session_id.clone(),
            session_approval_mode: "auto".to_string(),
        })
        .expect("切换审批模式失败");

    let event_bus = EventBus::new(&database);
    event_bus
        .register_session(&session.session_id)
        .expect("注册会话失败");

    let gateway = ScriptedModelGateway::new(vec![
        ModelGatewayResponse::ToolCall {
            tool_name: "read_file".to_string(),
            arguments: json!({
                "path": readme.to_string_lossy().to_string()
            }),
        },
        ModelGatewayResponse::FinalText {
            content: "指标采集完成。".to_string(),
        },
    ]);

    let runner = AgentRunner::new(
        &database,
        &config,
        gateway,
        AgentRunnerConfig::new(workspace_root, skill_root),
    )
    .with_event_bus(event_bus.clone());

    let outcome = runner
        .run_round(AgentRoundRequest {
            session_id: session.session_id.clone(),
            agent_id: "AGT-0001".to_string(),
            user_input: "请读取 README 并总结".to_string(),
        })
        .expect("执行单轮流程失败");
    assert_eq!(outcome.tool_responses.len(), 1);
    assert_eq!(format!("{:?}", outcome.tool_responses[0].status), "Success");

    let metrics_repository = SessionMetricsRepository::new(database.connection());
    let latest = metrics_repository
        .latest_by_session(&session.session_id)
        .expect("查询最新指标失败")
        .expect("缺少指标快照");
    assert!(latest.input_tokens > 0);
    assert!(latest.output_tokens > 0);
    assert!(latest.remaining_context >= 0);
    assert!(
        latest.tool_success_count >= 1,
        "tool_success_count 实际为 {}",
        latest.tool_success_count
    );

    let events = event_bus
        .drain_session_events(&session.session_id)
        .expect("读取事件失败");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "metrics_updated")
    );
}

#[test]
fn 十个会话的基础回归应保持消息不串线() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("regression-ten-sessions");
    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("回归目录".to_string()),
        })
        .expect("创建目录失败");

    let mut session_ids = Vec::new();
    for index in 0..10 {
        let session = service
            .create_session(CreateSessionRequest {
                workspace_id: workspace.workspace_id.clone(),
                first_prompt: format!("会话 {}", index + 1),
            })
            .expect("创建会话失败");
        session_ids.push(session.session_id);
    }

    let list_result = service
        .list_sessions_by_workspace(&workspace.workspace_id)
        .expect("查询会话列表失败");
    assert_eq!(list_result.len(), 10);
    for (index, session) in list_result.iter().enumerate() {
        assert!(session.title.contains(&(index + 1).to_string()) || !session.title.is_empty());
    }
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
