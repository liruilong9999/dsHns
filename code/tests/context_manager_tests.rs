//! 上下文管理器测试。
//!
//! 这些测试覆盖 `Token` 估算、压缩触发、压缩记录落盘与长结果预算策略。

use dshns_agent::app::workspace_session_service::{
    CreateSessionRequest, EnsureWorkspaceRequest, WorkspaceSessionService,
};
use dshns_agent::infra::config::{AppConfig, EnvSource};
use dshns_agent::infra::context_management::{
    CompressionReason, ContextManager, ContextManagerConfig, LongResultBudgetInput,
    LongResultStrategy,
};
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::repository::{ContextCompressionRepository, MessageRepository};
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
fn 超过上下文上限时应触发压缩并记录压缩结果() {
    let database = create_initialized_database();
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "test-key")]));
    let workspace_root = create_temp_directory("context-compress-over");
    let session_id = create_session(&database, &config, &workspace_root);
    let message_repository = MessageRepository::new(database.connection());

    for index in 0..6 {
        message_repository
            .create_runtime_message(
                &session_id,
                "",
                &format!("ROUND-10{index}"),
                if index % 2 == 0 { "user" } else { "assistant" },
                &format!(
                    "这是第 {} 条很长的可见消息{}",
                    index + 1,
                    "内容".repeat(200)
                ),
                "plain",
                true,
            )
            .expect("写入可见消息失败");
    }
    message_repository
        .create_command_audit_message(&session_id, "ROUND-CMD", "/sessions")
        .expect("写入命令审计失败");

    let context_manager = ContextManager::new(&database, ContextManagerConfig::default());

    let summary = context_manager
        .compress_session_context(&session_id, "AGT-0001", 2_800, CompressionReason::OverLimit)
        .expect("执行上下文压缩失败");

    assert!(summary.summary_text.contains("压缩摘要"));
    assert_eq!(summary.kept_messages.len(), 4);
    assert_eq!(summary.trigger_reason, CompressionReason::OverLimit);

    let compression_repository = ContextCompressionRepository::new(database.connection());
    let records = compression_repository
        .list_by_session_id(&session_id)
        .expect("查询压缩记录失败");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kept_message_count, 4);
    assert_eq!(records[0].trigger_reason, "over_limit");

    let messages = message_repository
        .list_by_session_id(&session_id)
        .expect("查询消息失败");
    let compressed_count = messages
        .iter()
        .filter(|message| message.is_compressed_source)
        .count();
    assert_eq!(compressed_count, 3);
}

#[test]
fn 超过百分之八十五阈值时应判定为近上限压缩() {
    let database = create_initialized_database();
    let context_manager = ContextManager::new(&database, ContextManagerConfig::default());

    let decision = context_manager.evaluate_compression_need(
        200_000,
        20_000,
        1_000,
        256_000,
        4_096,
        "deepseek-v4-flash",
    );

    assert_eq!(decision, Some(CompressionReason::NearLimit));
}

#[test]
fn 长失败结果和长成功结果应按预算选择策略() {
    let database = create_initialized_database();
    let context_manager = ContextManager::new(&database, ContextManagerConfig::default());

    let short_failure = context_manager.handle_long_result(LongResultBudgetInput {
        content: "短失败信息".repeat(20),
        is_failure: true,
        context_tokens_before_result: 250_000,
        max_context: 256_000,
        tool_tokens: 1_000,
        skill_tokens: 500,
        expected_output_tokens: 1_024,
        model_name: "deepseek-v4-flash".to_string(),
        summary_agent_available: false,
    });
    assert_eq!(
        short_failure.strategy,
        LongResultStrategy::DirectAfterCompression
    );

    let very_long_failure = context_manager.handle_long_result(LongResultBudgetInput {
        content: "超长失败信息".repeat(400),
        is_failure: true,
        context_tokens_before_result: 255_000,
        max_context: 256_000,
        tool_tokens: 1_000,
        skill_tokens: 500,
        expected_output_tokens: 1_024,
        model_name: "deepseek-v4-flash".to_string(),
        summary_agent_available: false,
    });
    assert_eq!(
        very_long_failure.strategy,
        LongResultStrategy::TruncateLast500Chars
    );
    assert!(very_long_failure.content.chars().count() <= 500);

    let long_success_with_summary = context_manager.handle_long_result(LongResultBudgetInput {
        content: "成功结果内容".repeat(250),
        is_failure: false,
        context_tokens_before_result: 250_000,
        max_context: 256_000,
        tool_tokens: 1_000,
        skill_tokens: 500,
        expected_output_tokens: 1_024,
        model_name: "deepseek-v4-flash".to_string(),
        summary_agent_available: true,
    });
    assert_eq!(
        long_success_with_summary.strategy,
        LongResultStrategy::SummaryGenerated
    );

    let long_success_without_summary = context_manager.handle_long_result(LongResultBudgetInput {
        content: "成功结果内容".repeat(250),
        is_failure: false,
        context_tokens_before_result: 250_000,
        max_context: 256_000,
        tool_tokens: 1_000,
        skill_tokens: 500,
        expected_output_tokens: 1_024,
        model_name: "deepseek-v4-flash".to_string(),
        summary_agent_available: false,
    });
    assert_eq!(
        long_success_without_summary.strategy,
        LongResultStrategy::TruncateLast500Chars
    );
}

fn create_initialized_database() -> SqliteDatabase {
    let database = SqliteDatabase::open(DatabaseTarget::InMemory).expect("打开内存数据库失败");
    database.initialize().expect("初始化数据库失败");
    database
}

fn create_session(
    database: &SqliteDatabase,
    config: &AppConfig,
    workspace_root: &PathBuf,
) -> String {
    let service = WorkspaceSessionService::new(database, config);
    let workspace = service
        .ensure_workspace(EnsureWorkspaceRequest {
            root_path: workspace_root.to_string_lossy().to_string(),
            display_name: Some("上下文目录".to_string()),
        })
        .expect("创建目录失败");
    service
        .create_session(CreateSessionRequest {
            workspace_id: workspace.workspace_id,
            first_prompt: "首句消息".to_string(),
        })
        .expect("创建会话失败")
        .session_id
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
