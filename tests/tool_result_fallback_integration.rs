//! 工具结果读取回退分支集成测试。

use std::path::PathBuf;
use std::sync::Arc;

use dshns::config::settings::Settings;
use dshns::domain::{ApprovalMode, Message, Session};
use dshns::persistence::sqlite::SqliteStore;
use dshns::session::manager::SessionManager;
use dshns::utils::fs::write_utf8;

#[test]
fn should_read_tool_result_by_call_id_from_file_fallback() {
    let workspace = PathBuf::from(format!(
        "target/test_tool_result_call_id_fallback_{}",
        uuid::Uuid::new_v4()
    ));
    let settings = Settings::load(&workspace).expect("加载测试配置失败");
    let store = Arc::new(SqliteStore::new(&settings.database_path).expect("创建数据库失败"));
    let manager = SessionManager::new(settings.clone(), store);

    let session_dir = settings.sessions_root.join("session-demo");
    let session = Session::new(
        "session-demo".to_string(),
        "directory-demo".to_string(),
        "demo".to_string(),
        "demo-project".to_string(),
        workspace.to_string_lossy().to_string(),
        workspace.to_string_lossy().to_string(),
        "deepseek-v4-flash".to_string(),
        ApprovalMode::AskUser,
        true,
        session_dir.clone(),
        "system prompt".to_string(),
    );
    manager
        .save_snapshot(&session, &[Message::user("hello")])
        .expect("保存会话失败");
    write_utf8(
        &session_dir.join("tool_results").join("index.json"),
        r#"[{"tool_call_id":"call_2","tool_name":"read_file","handle":"tool:call_2","body_file_path":"","projection_type":"InlineFull","projection_content":"hello-from-call-id","summary":"ok","preview_head":"","preview_tail":"","char_count":18,"byte_count":18,"success":true,"truncated":false,"externalized":false,"updated_at":"2026-01-01T00:00:00Z"}]"#,
    )
    .expect("写入工具结果索引失败");

    let result = manager
        .read_tool_result_by_call_id("session-demo", "call_2")
        .expect("按工具调用标识读取失败");
    assert_eq!(result, "hello-from-call-id");
}
