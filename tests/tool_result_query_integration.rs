//! 工具结果再次读取集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{HandleReadTool, RetrieveToolResultTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::{ensure_directory, write_utf8};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_retrieve_tool_result_and_read_handles() {
    let workspace_root = PathBuf::from(format!(
        "target/test_tool_result_query_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    let tool_results_dir = session_dir.join("tool_results");
    let body_path = tool_results_dir.join("call_demo.txt");
    let file_path = workspace_root.join("note.txt");

    ensure_directory(&tool_results_dir).expect("创建工具结果目录失败");
    write_utf8(
        &body_path,
        "alpha beta gamma delta epsilon zeta keyword omega",
    )
    .expect("写入工具结果正文失败");
    write_utf8(&file_path, "workspace-file-content").expect("写入工作区文件失败");
    write_utf8(
        &tool_results_dir.join("index.json"),
        &serde_json::to_string_pretty(&vec![json!({
            "tool_call_id": "call_demo",
            "tool_name": "run_shell",
            "handle": "tool:call_demo",
            "body_file_path": body_path.to_string_lossy(),
            "projection_type": "Summary",
            "projection_content": "projection",
            "summary": "summary-text",
            "preview_head": "alpha beta",
            "preview_tail": "keyword omega",
            "char_count": 49,
            "byte_count": 49,
            "success": true,
            "truncated": true,
            "externalized": true,
            "updated_at": "2026-01-01T00:00:00Z"
        })])
        .expect("序列化工具结果索引失败"),
    )
    .expect("写入工具结果索引失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let retrieve_tool = RetrieveToolResultTool;
    let summary = retrieve_tool
        .handle(
            json!({
                "tool_call_id": "call_demo",
                "mode": "summary"
            }),
            &context,
        )
        .await
        .expect("读取工具结果摘要失败");
    assert_eq!(summary, "summary-text");

    let slice = retrieve_tool
        .handle(
            json!({
                "tool_call_id": "call_demo",
                "mode": "slice",
                "start_char": 6,
                "length_chars": 10
            }),
            &context,
        )
        .await
        .expect("读取工具结果切片失败");
    assert_eq!(slice, "beta gamma");

    let keyword_context = retrieve_tool
        .handle(
            json!({
                "tool_call_id": "call_demo",
                "mode": "keyword_context",
                "keyword": "keyword",
                "context_chars": 6
            }),
            &context,
        )
        .await
        .expect("读取关键字上下文失败");
    assert!(keyword_context.contains("keyword"));

    let handle_tool = HandleReadTool;
    let tool_body = handle_tool
        .handle(
            json!({
                "handle": "tool:call_demo",
                "max_chars": 5
            }),
            &context,
        )
        .await
        .expect("读取 tool 句柄失败");
    assert_eq!(tool_body, "alpha");

    let file_body = handle_tool
        .handle(
            json!({
                "handle": format!("file:{}", file_path.strip_prefix(&workspace_root).expect("计算相对路径失败").to_string_lossy()),
                "max_chars": 9
            }),
            &context,
        )
        .await
        .expect("读取 file 句柄失败");
    assert_eq!(file_body, "workspace");
}
