//! Tauri IPC 预留契约测试。
//!
//! 这些测试覆盖目录/会话预留命令结构，以及流式订阅的当前阶段未实现响应。

use dshns_agent::app::workspace_session_service::WorkspaceSessionService;
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::event_bus::EventBus;
use dshns_agent::ipc::tauri_contract::{
    SessionCreateRequest, SessionListRequest, SessionSendMessageRequest,
    SessionSubscribeStreamRequest, SessionSwitchApprovalModeRequest, SessionSwitchModelRequest,
    SessionUpdateRequest, TauriIpcFacade, WorkspaceCreateRequest, WorkspaceDeleteRequest,
    WorkspaceUpdateRequest,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
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

#[test]
fn 应支持目录与会话_ipc_预留命令的最小闭环() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("ipc-contract-workspace");
    let event_bus = EventBus::new(&database);
    let service = WorkspaceSessionService::new(&database, &config);
    let facade = TauriIpcFacade::new(service, event_bus);

    let workspace_created = facade
        .workspace_create(WorkspaceCreateRequest {
            workspace_path: workspace_root.to_string_lossy().to_string(),
            name: "IPC 测试目录".to_string(),
        })
        .expect("workspace_create 执行失败");
    assert!(workspace_created.ok);

    let workspace_updated = facade
        .workspace_update(WorkspaceUpdateRequest {
            workspace_id: workspace_created.workspace_id.clone(),
            name: "IPC 重命名目录".to_string(),
        })
        .expect("workspace_update 执行失败");
    assert_eq!(workspace_updated.name, "IPC 重命名目录");

    let session_created = facade
        .session_create(SessionCreateRequest {
            workspace_id: workspace_created.workspace_id.clone(),
            first_prompt: "请创建一个 IPC 会话".to_string(),
        })
        .expect("session_create 执行失败");
    assert!(session_created.message_enqueued);

    let sessions = facade
        .session_list(SessionListRequest {
            workspace_id: workspace_created.workspace_id.clone(),
        })
        .expect("session_list 执行失败");
    assert_eq!(sessions.sessions.len(), 1);

    let session_updated = facade
        .session_update(SessionUpdateRequest {
            session_id: session_created.session_id.clone(),
            title: "新的 IPC 会话标题".to_string(),
        })
        .expect("session_update 执行失败");
    assert_eq!(session_updated.title, "新的 IPC 会话标题");

    let sent_message = facade
        .session_send_message(SessionSendMessageRequest {
            session_id: session_created.session_id.clone(),
            content: "请继续追加一条消息".to_string(),
        })
        .expect("session_send_message 执行失败");
    assert!(sent_message.message_enqueued);
    assert!(sent_message.event_id.starts_with("EVT-"));

    let switched_model = facade
        .session_switch_model(SessionSwitchModelRequest {
            session_id: session_created.session_id.clone(),
            model_name: "deepseek-v4-pro".to_string(),
        })
        .expect("session_switch_model 执行失败");
    assert_eq!(switched_model.current_model, "deepseek-v4-pro");

    let switched_mode = facade
        .session_switch_approval_mode(SessionSwitchApprovalModeRequest {
            session_id: session_created.session_id.clone(),
            target_mode: "auto".to_string(),
        })
        .expect("session_switch_approval_mode 执行失败");
    assert_eq!(switched_mode.session_approval_mode, "auto");

    let deleted = facade
        .workspace_delete(WorkspaceDeleteRequest {
            workspace_id: workspace_created.workspace_id,
        })
        .expect("workspace_delete 执行失败");
    assert!(deleted.deleted);
}

#[test]
fn 流式订阅_ipc_当前阶段应返回未实现错误() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let event_bus = EventBus::new(&database);
    let service = WorkspaceSessionService::new(&database, &config);
    let facade = TauriIpcFacade::new(service, event_bus);

    let subscribe_result = facade.session_subscribe_stream(SessionSubscribeStreamRequest {
        session_id: "SES-0001".to_string(),
    });

    let error = subscribe_result.expect_err("当前阶段流式订阅应返回未实现错误");
    assert_eq!(error.error_code, "NOT_IMPLEMENTED");
    assert!(error.message.contains("尚未实现前端订阅"));
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
