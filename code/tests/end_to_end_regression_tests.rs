//! 端到端回归测试。
//!
//! 这些测试覆盖从 IPC 预留命令、事件总线、单轮执行到指标快照的一条完整链路。

use dshns_agent::app::agent_runner::{
    AgentRoundRequest, AgentRunner, AgentRunnerConfig, ModelGatewayError, ModelGatewayRequest,
    ModelGatewayResponse, ModelGatewayTrait,
};
use dshns_agent::app::workspace_session_service::WorkspaceSessionService;
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::event_bus::EventBus;
use dshns_agent::infra::metrics::SessionMetricsRepository;
use dshns_agent::infra::repository::{EventLogRepository, MessageRepository};
use dshns_agent::ipc::tauri_contract::{
    SessionCreateRequest, SessionListRequest, SessionSendMessageRequest,
    SessionSwitchApprovalModeRequest, SessionSwitchModelRequest, TauriIpcFacade,
    WorkspaceCreateRequest,
};
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
fn 端到端回归应覆盖_ipc_消息处理事件与指标快照() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("e2e-workspace");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");
    let target_file = workspace_root.join("README.md");
    fs::write(&target_file, "端到端回归测试文件内容。").expect("写入 README 失败");

    let event_bus = EventBus::new(&database);
    let service = WorkspaceSessionService::new(&database, &config);
    let facade = TauriIpcFacade::new(service, event_bus.clone());

    let workspace = facade
        .workspace_create(WorkspaceCreateRequest {
            workspace_path: workspace_root.to_string_lossy().to_string(),
            name: "端到端目录".to_string(),
        })
        .expect("workspace_create 失败");
    let session = facade
        .session_create(SessionCreateRequest {
            workspace_id: workspace.workspace_id.clone(),
            first_prompt: "请创建端到端会话".to_string(),
        })
        .expect("session_create 失败");
    let _ = facade
        .session_switch_model(SessionSwitchModelRequest {
            session_id: session.session_id.clone(),
            model_name: "deepseek-v4-pro".to_string(),
        })
        .expect("session_switch_model 失败");
    let _ = facade
        .session_switch_approval_mode(SessionSwitchApprovalModeRequest {
            session_id: session.session_id.clone(),
            target_mode: "auto".to_string(),
        })
        .expect("session_switch_approval_mode 失败");
    let send_result = facade
        .session_send_message(SessionSendMessageRequest {
            session_id: session.session_id.clone(),
            content: "请准备执行端到端主流程".to_string(),
        })
        .expect("session_send_message 失败");
    assert!(send_result.event_id.starts_with("EVT-"));

    let gateway = ScriptedModelGateway::new(vec![
        ModelGatewayResponse::ToolCall {
            tool_name: "read_file".to_string(),
            arguments: json!({
                "path": target_file.to_string_lossy().to_string()
            }),
        },
        ModelGatewayResponse::FinalText {
            content: "端到端流程执行完成。".to_string(),
        },
    ]);
    event_bus
        .register_session(&session.session_id)
        .expect("注册会话失败");
    let runner = AgentRunner::new(
        &database,
        &config,
        gateway,
        AgentRunnerConfig::new(workspace_root.clone(), skill_root),
    )
    .with_event_bus(event_bus.clone());

    let outcome = runner
        .run_round(AgentRoundRequest {
            session_id: session.session_id.clone(),
            agent_id: "AGT-0001".to_string(),
            user_input: "请读取 README 并给出总结".to_string(),
        })
        .expect("执行单轮流程失败");
    assert_eq!(outcome.final_text.as_deref(), Some("端到端流程执行完成。"));

    let list_result = facade
        .session_list(SessionListRequest {
            workspace_id: workspace.workspace_id,
        })
        .expect("session_list 失败");
    assert_eq!(list_result.sessions.len(), 1);

    let message_repository = MessageRepository::new(database.connection());
    let messages = message_repository
        .list_by_session_id(&session.session_id)
        .expect("查询消息失败");
    assert!(messages.iter().any(|message| message.role == "tool"));
    assert!(messages.iter().any(|message| message.role == "assistant"));

    let metrics_repository = SessionMetricsRepository::new(database.connection());
    let latest_metric = metrics_repository
        .latest_by_session(&session.session_id)
        .expect("查询指标失败")
        .expect("缺少指标快照");
    assert!(latest_metric.output_tokens > 0);

    let event_repository = EventLogRepository::new(database.connection());
    let event_logs = event_repository
        .list_by_session(&session.session_id)
        .expect("查询事件日志失败");
    assert!(
        event_logs
            .iter()
            .any(|event| event.event_type == "assistant_output_completed")
    );
    assert!(
        event_logs
            .iter()
            .any(|event| event.event_type == "metrics_updated")
    );
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
