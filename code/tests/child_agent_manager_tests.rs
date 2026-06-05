//! 子智能体管理测试。
//!
//! 这些测试覆盖创建、派发、销毁、继承/隔离模式，以及 `create_agent` 工具接入。

use dshns_agent::app::workspace_session_service::{
    CreateSessionRequest, EnsureWorkspaceRequest, WorkspaceSessionService,
};
use dshns_agent::domain::tool::{SessionApprovalMode, ToolCallRequest, ToolExecutionStatus};
use dshns_agent::infra::agent_management::{
    ChildAgentDispatchRequest, ChildAgentManager, ChildAgentManagerConfig, ChildAgentMode,
    CreateChildAgentRequest,
};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::event_bus::EventBus;
use dshns_agent::infra::repository::{AgentRelationRepository, AgentRepository};
use dshns_agent::infra::tool_system::{ToolDispatcher, ToolRuntimeConfig};
use serde_json::json;
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
fn 应支持继承模式与隔离模式创建派发和销毁() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("child-agent-workspace");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");
    let event_bus = EventBus::new(&database);

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("子智能体目录".to_string()),
        })
        .expect("创建目录失败");
    let session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id.clone(),
            first_prompt: "父会话首条消息".to_string(),
        })
        .expect("创建父会话失败");

    let manager = ChildAgentManager::new(
        &database,
        event_bus.clone(),
        ChildAgentManagerConfig::default(),
    );

    let inherit_child = manager
        .create_child_agent(CreateChildAgentRequest {
            parent_session_id: session.session_id.clone(),
            parent_agent_id: "AGT-PARENT".to_string(),
            mode: ChildAgentMode::Inherit,
            task_summary: "继承模式任务".to_string(),
            inherited_context: Some("父上下文摘要".to_string()),
        })
        .expect("创建继承模式子智能体失败");
    assert_eq!(inherit_child.child_session_id, session.session_id);
    assert_eq!(inherit_child.mode, ChildAgentMode::Inherit);

    let isolated_child = manager
        .create_child_agent(CreateChildAgentRequest {
            parent_session_id: session.session_id.clone(),
            parent_agent_id: "AGT-PARENT".to_string(),
            mode: ChildAgentMode::Isolated,
            task_summary: "隔离模式任务".to_string(),
            inherited_context: None,
        })
        .expect("创建隔离模式子智能体失败");
    assert_ne!(isolated_child.child_session_id, session.session_id);
    assert_eq!(isolated_child.mode, ChildAgentMode::Isolated);

    let dispatch_result = manager
        .dispatch_child_agent(ChildAgentDispatchRequest {
            child_agent_id: inherit_child.child_agent_id.clone(),
            task_summary: "继续派发第二个任务".to_string(),
            result_summary: Some("当前任务执行完成".to_string()),
        })
        .expect("继续派发子智能体失败");
    assert_eq!(dispatch_result.current_status, "waiting");

    let destroy_result = manager
        .destroy_child_agent(&inherit_child.child_agent_id)
        .expect("销毁子智能体失败");
    assert_eq!(destroy_result.current_status, "destroyed");

    let agent_repository = AgentRepository::new(database.connection());
    let child_record = agent_repository
        .get_by_id(&isolated_child.child_agent_id)
        .expect("查询子智能体失败")
        .expect("子智能体不存在");
    assert_eq!(child_record.status, "created");

    let relation_repository = AgentRelationRepository::new(database.connection());
    let relations = relation_repository
        .list_by_parent_agent("AGT-PARENT")
        .expect("查询父子关系失败");
    assert_eq!(relations.len(), 2);
}

#[test]
fn 超过层级或数量限制时应拒绝创建() {
    let database = create_initialized_database();
    let event_bus = EventBus::new(&database);
    let manager = ChildAgentManager::new(
        &database,
        event_bus,
        ChildAgentManagerConfig {
            max_child_depth: 0,
            max_child_agents_total: 1,
        },
    );

    let depth_error = manager.create_child_agent(CreateChildAgentRequest {
        parent_session_id: "SES-0001".to_string(),
        parent_agent_id: "AGT-PARENT".to_string(),
        mode: ChildAgentMode::Inherit,
        task_summary: "超深度".to_string(),
        inherited_context: None,
    });
    assert!(depth_error.is_err());
}

#[test]
fn create_agent_工具应支持创建继续派发和销毁() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("child-agent-tool-workspace");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");
    let event_bus = EventBus::new(&database);

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("工具目录".to_string()),
        })
        .expect("创建目录失败");
    let session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id,
            first_prompt: "父会话".to_string(),
        })
        .expect("创建会话失败");

    let manager = ChildAgentManager::new(&database, event_bus, ChildAgentManagerConfig::default());
    let mut dispatcher = ToolDispatcher::new(ToolRuntimeConfig::new(workspace_root, skill_root))
        .with_child_agent_manager(manager);

    let create_response = dispatcher.execute(
        ToolCallRequest::new(
            "create_agent",
            &session.session_id,
            "AGT-PARENT",
            "ROUND-0100",
            json!({
                "action": "create",
                "mode": "isolated",
                "task": "请评审需求文档"
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_eq!(create_response.status, ToolExecutionStatus::Success);
    let child_agent_id = create_response.result_payload["child_agent_id"]
        .as_str()
        .expect("缺少 child_agent_id")
        .to_string();

    let dispatch_response = dispatcher.execute(
        ToolCallRequest::new(
            "create_agent",
            &session.session_id,
            "AGT-PARENT",
            "ROUND-0101",
            json!({
                "action": "dispatch",
                "child_agent_id": child_agent_id,
                "task": "请继续补充评审结果"
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_eq!(dispatch_response.status, ToolExecutionStatus::Success);
    assert_eq!(
        dispatch_response.result_payload["status"].as_str(),
        Some("waiting")
    );

    let destroy_response = dispatcher.execute(
        ToolCallRequest::new(
            "create_agent",
            &session.session_id,
            "AGT-PARENT",
            "ROUND-0102",
            json!({
                "action": "destroy",
                "child_agent_id": dispatch_response.result_payload["child_agent_id"].as_str().unwrap()
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_eq!(destroy_response.status, ToolExecutionStatus::Success);
    assert_eq!(
        destroy_response.result_payload["status"].as_str(),
        Some("destroyed")
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
