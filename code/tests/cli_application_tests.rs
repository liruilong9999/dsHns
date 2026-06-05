//! CLI 应用层测试。
//!
//! 这些测试覆盖普通输入处理、基础命令解析、命令审计过滤与显示状态定义。

use dshns_agent::app::cli::{CliApplication, CliDisplayState, CliResponse};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::repository::MessageRepository;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 测试用环境变量读取器。
struct FakeEnvSource {
    /// 预置环境变量键值。
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
fn 普通输入应在首次处理时创建会话并接受后续消息() {
    let database = create_initialized_database();
    let config = create_test_config();
    let workspace_path = create_temp_workspace_path("cli-normal-input");
    let mut cli = CliApplication::new(
        &database,
        &config,
        workspace_path.to_string_lossy().to_string(),
    );

    let first_response = cli
        .handle_input("请帮我初始化当前项目分析")
        .expect("首次普通输入失败");
    match first_response {
        CliResponse::TextAccepted {
            created_new_session,
            session_id,
            ..
        } => {
            assert!(created_new_session);
            assert!(session_id.starts_with("SES-"));
        }
        other => panic!("首次普通输入返回类型不正确：{other:?}"),
    }

    let second_response = cli
        .handle_input("继续补充第二条消息")
        .expect("第二次普通输入失败");
    match second_response {
        CliResponse::TextAccepted {
            created_new_session,
            round_id,
            ..
        } => {
            assert!(!created_new_session);
            assert!(round_id.starts_with("ROUND-"));
        }
        other => panic!("第二次普通输入返回类型不正确：{other:?}"),
    }
}

#[test]
fn 应支持模型查询切换审批模式会话查询与退出命令() {
    let database = create_initialized_database();
    let config = create_test_config();
    let workspace_path = create_temp_workspace_path("cli-commands");
    let mut cli = CliApplication::new(
        &database,
        &config,
        workspace_path.to_string_lossy().to_string(),
    );

    cli.handle_input("请先创建一个会话")
        .expect("创建基础会话失败");

    let models_response = cli.handle_input("/models").expect("执行 /models 失败");
    match models_response {
        CliResponse::ModelsListed { models } => {
            assert_eq!(models.len(), 4);
            assert!(models.contains(&"deepseek-v4-pro".to_string()));
        }
        other => panic!("/models 返回类型不正确：{other:?}"),
    }

    let model_response = cli
        .handle_input("/model check deepseek-v4-pro")
        .expect("执行 /model check 失败");
    match model_response {
        CliResponse::ModelChanged {
            current_model,
            context_limit,
            ..
        } => {
            assert_eq!(current_model, "deepseek-v4-pro");
            assert_eq!(context_limit, 256_000);
        }
        other => panic!("/model check 返回类型不正确：{other:?}"),
    }

    let first_mode = cli.handle_input("/mode").expect("第一次 /mode 失败");
    let second_mode = cli.handle_input("/mode").expect("第二次 /mode 失败");
    let third_mode = cli.handle_input("/mode").expect("第三次 /mode 失败");
    let explicit_mode = cli
        .handle_input("/mode auto")
        .expect("显式 /mode auto 失败");

    assert_mode(first_mode, "auto");
    assert_mode(second_mode, "allow_all");
    assert_mode(third_mode, "ask");
    assert_mode(explicit_mode, "auto");

    let sessions_response = cli.handle_input("/sessions").expect("执行 /sessions 失败");
    match sessions_response {
        CliResponse::SessionsListed {
            workspace_id,
            sessions,
        } => {
            assert!(workspace_id.starts_with("WS-"));
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].title, "请先创建一个会话");
        }
        other => panic!("/sessions 返回类型不正确：{other:?}"),
    }

    let quit_response = cli.handle_input("/quit").expect("执行 /quit 失败");
    assert!(matches!(quit_response, CliResponse::Quit { quit: true }));
}

#[test]
fn 命令审计应入库但不得进入模型上下文() {
    let database = create_initialized_database();
    let config = create_test_config();
    let workspace_path = create_temp_workspace_path("cli-command-audit");
    let mut cli = CliApplication::new(
        &database,
        &config,
        workspace_path.to_string_lossy().to_string(),
    );

    let created = cli.handle_input("先创建审计会话").expect("创建会话失败");
    let session_id = match created {
        CliResponse::TextAccepted { session_id, .. } => session_id,
        other => panic!("会话创建返回类型不正确：{other:?}"),
    };

    cli.handle_input("/sessions").expect("执行 /sessions 失败");

    let message_repository = MessageRepository::new(database.connection());
    let messages = message_repository
        .list_by_session_id(&session_id)
        .expect("查询会话消息失败");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].content_type, "command_audit");
    assert!(!messages[1].include_in_context);
}

#[test]
fn 显示状态应提供颜色与标签区分() {
    assert_eq!(CliDisplayState::Thinking.color_name(), "orange");
    assert_eq!(CliDisplayState::Thinking.prefix_label(), "[思考]");
    assert_eq!(CliDisplayState::ToolRunning.color_name(), "white");
    assert_eq!(CliDisplayState::ToolRunning.prefix_label(), "[工具执行中]");
    assert_eq!(CliDisplayState::ToolSuccess.color_name(), "green");
    assert_eq!(CliDisplayState::ToolFailure.color_name(), "red");
    assert_eq!(CliDisplayState::Answer.prefix_label(), "[回答]");
}

/// 断言审批模式切换结果。
fn assert_mode(response: CliResponse, expected_mode: &str) {
    match response {
        CliResponse::ModeChanged {
            session_approval_mode,
            ..
        } => {
            assert_eq!(session_approval_mode, expected_mode);
        }
        other => panic!("/mode 返回类型不正确：{other:?}"),
    }
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
    let path = std::env::temp_dir().join(format!("dshns-cli-{prefix}-{unique_suffix}"));
    fs::create_dir_all(&path).expect("创建测试目录失败");
    path
}
