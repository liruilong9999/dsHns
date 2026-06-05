//! 智能体单轮执行测试。
//!
//! 这些测试覆盖模型网关可用性校验与“一轮输入到最终输出”的主流程。

use dshns_agent::app::agent_runner::{
    AgentRoundRequest, AgentRoundStatus, AgentRunner, AgentRunnerConfig, ModelGatewayError,
    ModelGatewayRequest, ModelGatewayResponse, ModelGatewayTrait,
};
use dshns_agent::app::workspace_session_service::{
    CreateSessionRequest, EnsureWorkspaceRequest, WorkspaceSessionService,
};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
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
        if responses.is_empty() {
            return Err(ModelGatewayError::RequestFailed(
                "模型脚本响应已耗尽。".to_string(),
            ));
        }

        Ok(responses.remove(0))
    }
}

#[test]
fn api_key_缺失时应阻止模型请求并返回中文错误() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([]));
    let workspace_root = create_temp_directory("agent-runner-missing-key");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("测试目录".to_string()),
        })
        .expect("创建目录失败");
    let session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id,
            first_prompt: "第一句提示词".to_string(),
        })
        .expect("创建会话失败");

    let gateway = ScriptedModelGateway::new(vec![ModelGatewayResponse::FinalText {
        content: "不会被使用".to_string(),
    }]);
    let runner = AgentRunner::new(
        &database,
        &config,
        gateway,
        AgentRunnerConfig::new(workspace_root, skill_root),
    );

    let error = runner
        .run_round(AgentRoundRequest {
            session_id: session.session_id,
            agent_id: "AGT-0001".to_string(),
            user_input: "请继续执行".to_string(),
        })
        .expect_err("缺失 API Key 时不应继续执行");

    assert!(error.to_string().contains("环境变量 DEEPSEEK_API_KEY 缺失"));
}

#[test]
fn 应支持一轮输入到工具执行再到最终输出() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("agent-runner-success");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");
    fs::write(workspace_root.join("AGENTS.md"), "工作区约束").expect("写入工作区 AGENTS 失败");
    let target_file = workspace_root.join("demo.txt");
    fs::write(&target_file, "这是工具读取结果。").expect("写入测试文件失败");

    let service = WorkspaceSessionService::new(&database, &config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("测试目录".to_string()),
        })
        .expect("创建目录失败");
    let session = service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id,
            first_prompt: "第一句提示词".to_string(),
        })
        .expect("创建会话失败");

    let gateway = ScriptedModelGateway::new(vec![
        ModelGatewayResponse::ToolCall {
            tool_name: "read_file".to_string(),
            arguments: json!({
                "path": target_file.to_string_lossy().to_string()
            }),
        },
        ModelGatewayResponse::FinalText {
            content: "我已经读取文件并完成总结。".to_string(),
        },
    ]);
    let runner = AgentRunner::new(
        &database,
        &config,
        gateway,
        AgentRunnerConfig::new(workspace_root.clone(), skill_root.clone()),
    );

    let outcome = runner
        .run_round(AgentRoundRequest {
            session_id: session.session_id,
            agent_id: "AGT-0001".to_string(),
            user_input: "请读取 demo.txt 并总结".to_string(),
        })
        .expect("执行单轮主流程失败");

    assert_eq!(outcome.status, AgentRoundStatus::Completed);
    assert_eq!(
        outcome.final_text.as_deref(),
        Some("我已经读取文件并完成总结。")
    );
    assert!(
        outcome
            .state_history
            .contains(&"DispatchingTool".to_string())
    );
    assert!(
        outcome
            .tool_responses
            .iter()
            .any(|response| response.tool_name == "read_file")
    );
    assert!(outcome.prompt_snapshot.contains("工作区约束"));
    assert!(outcome.prompt_snapshot.contains("第一句提示词"));
    assert!(outcome.prompt_snapshot.contains("请读取 demo.txt 并总结"));
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
