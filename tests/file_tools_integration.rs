//! 基础文件与搜索工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{EditFileTool, FileSearchTool, GrepFilesTool, ListDirTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_list_search_grep_and_edit_files() {
    let workspace_root = PathBuf::from(format!(
        "target/test_file_tools_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    let src_dir = workspace_root.join("src");
    let nested_dir = src_dir.join("nested");
    ensure_directory(&session_dir).expect("创建会话目录失败");
    ensure_directory(&nested_dir).expect("创建嵌套目录失败");

    let readme_path = workspace_root.join("README.md");
    let rust_path = nested_dir.join("demo.rs");
    write_utf8(&readme_path, "hello rust workspace\nkeyword-line\n").expect("写入 README 失败");
    write_utf8(&rust_path, "fn main() {\n    println!(\"hello rust\");\n}\n")
        .expect("写入 Rust 文件失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let list_dir_tool = ListDirTool;
    let listed = list_dir_tool
        .handle(
            json!({
                "path": "src",
                "include_hidden": false
            }),
            &context,
        )
        .await
        .expect("list_dir 执行失败");
    assert!(listed.contains("nested"));

    let file_search_tool = FileSearchTool;
    let searched = file_search_tool
        .handle(
            json!({
                "query": "readme",
                "limit": 10
            }),
            &context,
        )
        .await
        .expect("file_search 执行失败");
    assert!(searched.to_ascii_lowercase().contains("readme.md"));

    let grep_tool = GrepFilesTool;
    let grep_result = grep_tool
        .handle(
            json!({
                "pattern": "hello\\s+rust",
                "limit": 10
            }),
            &context,
        )
        .await
        .expect("grep_files 执行失败");
    assert!(grep_result.contains("hello rust"));

    let edit_tool = EditFileTool;
    edit_tool
        .handle(
            json!({
                "path": "README.md",
                "old_text": "keyword-line",
                "new_text": "updated-line",
                "replace_all": false
            }),
            &context,
        )
        .await
        .expect("edit_file 执行失败");
    let edited = read_optional_utf8(&readme_path)
        .expect("读取编辑后的 README 失败")
        .expect("编辑后的 README 不存在");
    assert!(edited.contains("updated-line"));
}
