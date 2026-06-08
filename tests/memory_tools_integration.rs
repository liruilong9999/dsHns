//! 轻量记忆与编辑工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{FimEditTool, NoteTool, NotifyTool, RecallArchiveTool, RememberTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_write_and_recall_memory_tools() {
    let workspace_root = PathBuf::from(format!(
        "target/test_memory_tools_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    ensure_directory(&session_dir).expect("创建会话目录失败");
    let file_path = workspace_root.join("demo.txt");
    write_utf8(&file_path, "old-content").expect("写入 demo.txt 失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir: session_dir.clone(),
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    NoteTool
        .handle(
            json!({
                "title": "todo",
                "content": "临时备注内容"
            }),
            &context,
        )
        .await
        .expect("note 执行失败");

    RememberTool
        .handle(
            json!({
                "key": "profile",
                "content": "长期记忆内容"
            }),
            &context,
        )
        .await
        .expect("remember 执行失败");

    let recalled = RecallArchiveTool
        .handle(
            json!({
                "query": "记忆",
                "limit": 10
            }),
            &context,
        )
        .await
        .expect("recall_archive 执行失败");
    assert!(recalled.contains("长期记忆内容"));

    let notify_result = NotifyTool
        .handle(
            json!({
                "message": "通知测试"
            }),
            &context,
        )
        .await
        .expect("notify 执行失败");
    assert!(notify_result.contains("已发送终端通知"));

    FimEditTool
        .handle(
            json!({
                "path": "demo.txt",
                "old_text": "old-content",
                "new_text": "new-content"
            }),
            &context,
        )
        .await
        .expect("fim_edit 执行失败");
    let edited = read_optional_utf8(&file_path)
        .expect("读取编辑后的 demo.txt 失败")
        .expect("编辑后的 demo.txt 不存在");
    assert_eq!(edited, "new-content");
}
