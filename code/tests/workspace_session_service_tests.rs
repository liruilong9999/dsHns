//! 目录与会话服务集成测试。
//!
//! 这些测试覆盖 `TASK-005` 到 `TASK-007` 的仓储与服务最小闭环。

use dshns_agent::app::workspace_session_service::{
    CreateSessionRequest, EnsureWorkspaceRequest, RenameSessionRequest, RenameWorkspaceRequest,
    WorkspaceSessionService,
};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::repository::{MessageRepository, SessionRepository, WorkspaceRepository};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 测试用环境变量读取器。
struct FakeEnvSource {
    /// 预置环境变量内容。
    values: HashMap<String, String>,
}

impl FakeEnvSource {
    /// 构造测试环境变量读取器。
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
fn 应支持目录自动注册复用更新与逻辑删除() {
    let database = create_initialized_database();
    let config = create_test_config();
    let workspace_root = create_temp_workspace_path("workspace-lifecycle");

    let service = WorkspaceSessionService::new(&database, &config);
    let created = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: None,
        })
        .expect("创建目录失败");

    assert!(created.workspace_id.starts_with("WS-"));
    assert_eq!(
        created.name,
        workspace_root.file_name().unwrap().to_string_lossy()
    );

    let reused = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("不会重复创建".to_string()),
        })
        .expect("复用目录失败");
    assert_eq!(created.workspace_id, reused.workspace_id);

    let renamed = service
        .rename_workspace(RenameWorkspaceRequest {
            workspace_id: created.workspace_id.clone(),
            name: "新的项目名称".to_string(),
        })
        .expect("更新目录名称失败");
    assert_eq!(renamed.name, "新的项目名称");

    let deleted = service
        .delete_workspace(&created.workspace_id)
        .expect("逻辑删除目录失败");
    assert!(deleted.deleted);

    let workspace_repository = WorkspaceRepository::new(database.connection());
    let stored = workspace_repository
        .get_by_id_including_deleted(&created.workspace_id)
        .expect("查询目录失败")
        .expect("目录不存在");
    assert!(stored.is_deleted);
}

#[test]
fn 应在首句提示词时创建会话并写入首条消息() {
    let database = create_initialized_database();
    let config = create_test_config();
    let workspace_root = create_temp_workspace_path("session-create");

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("会话目录".to_string()),
        })
        .expect("创建目录失败");

    let created_session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id.clone(),
            first_prompt: "请帮我初始化当前项目分析".to_string(),
        })
        .expect("创建会话失败");

    assert!(created_session.session_id.starts_with("SES-"));
    assert!(created_session.round_id.starts_with("ROUND-"));
    assert_eq!(created_session.title, "请帮我初始化当前项目分析");
    assert!(created_session.message_enqueued);

    let session_repository = SessionRepository::new(database.connection());
    let stored_session = session_repository
        .get_by_id(&created_session.session_id)
        .expect("查询会话失败")
        .expect("会话不存在");
    assert_eq!(stored_session.workspace_id, workspace.workspace_id);
    assert_eq!(stored_session.title, created_session.title);
    assert_eq!(stored_session.current_model, "deepseek-v4-flash");
    assert_eq!(stored_session.session_approval_mode, "ask");
    assert_eq!(stored_session.context_limit, 256_000);
    assert!(stored_session.last_message_at.is_some());

    let message_repository = MessageRepository::new(database.connection());
    let messages = message_repository
        .list_by_session_id(&created_session.session_id)
        .expect("查询消息失败");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "请帮我初始化当前项目分析");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content_type, "plain");
    assert_eq!(messages[0].sequence_no, 1);
    assert!(messages[0].include_in_context);
    assert_eq!(messages[0].round_id, created_session.round_id);
}

#[test]
fn 应支持会话重命名并按目录查询会话列表() {
    let database = create_initialized_database();
    let config = create_test_config();
    let first_root = create_temp_workspace_path("workspace-a");
    let second_root = create_temp_workspace_path("workspace-b");

    let service = WorkspaceSessionService::new(&database, &config);
    let first_workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: first_root.to_string_lossy().to_string(),
            display_name: Some("目录A".to_string()),
        })
        .expect("创建目录A失败");
    let second_workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: second_root.to_string_lossy().to_string(),
            display_name: Some("目录B".to_string()),
        })
        .expect("创建目录B失败");

    let first_session = service
        .create_session(CreateSessionRequest {
            workspace_id: first_workspace.workspace_id.clone(),
            first_prompt: "第一个会话".to_string(),
        })
        .expect("创建第一个会话失败");
    service
        .create_session(CreateSessionRequest {
            workspace_id: second_workspace.workspace_id.clone(),
            first_prompt: "第二个会话".to_string(),
        })
        .expect("创建第二个会话失败");

    let renamed = service
        .rename_session(RenameSessionRequest {
            session_id: first_session.session_id.clone(),
            title: "新的会话标题".to_string(),
        })
        .expect("重命名会话失败");
    assert_eq!(renamed.title, "新的会话标题");

    let sessions = service
        .list_sessions_by_workspace(&first_workspace.workspace_id)
        .expect("查询目录会话失败");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, first_session.session_id);
    assert_eq!(sessions[0].title, "新的会话标题");
    assert_eq!(sessions[0].status, "active");
    assert_eq!(sessions[0].current_model, "deepseek-v4-flash");
}

/// 创建已经完成迁移的测试数据库。
fn create_initialized_database() -> SqliteDatabase {
    let database = SqliteDatabase::open(DatabaseTarget::InMemory).expect("打开内存数据库失败");
    database.initialize().expect("初始化数据库失败");
    database
}

/// 创建测试配置。
fn create_test_config() -> AppConfig {
    AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]))
}

/// 创建测试工作区目录路径。
fn create_temp_workspace_path(prefix: &str) -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于纪元")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("dshns-{prefix}-{unique_suffix}"));
    fs::create_dir_all(&path).expect("创建测试目录失败");
    path
}
