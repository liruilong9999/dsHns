//! Git 工具集成测试。
use std::path::PathBuf;
use std::process::Command;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{GitBlameTool, GitDiffTool, GitLogTool, GitShowTool, GitStatusTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::{ensure_directory, write_utf8};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_run_git_tools() {
    let workspace_root = PathBuf::from(format!(
        "target/test_git_tools_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    ensure_directory(&session_dir).expect("创建会话目录失败");

    Command::new("git")
        .args(["init"])
        .current_dir(&workspace_root)
        .output()
        .expect("初始化 Git 仓库失败");
    Command::new("git")
        .args(["config", "user.email", "codex@example.com"])
        .current_dir(&workspace_root)
        .output()
        .expect("配置 Git 邮箱失败");
    Command::new("git")
        .args(["config", "user.name", "Codex"])
        .current_dir(&workspace_root)
        .output()
        .expect("配置 Git 用户名失败");

    let file_path = workspace_root.join("demo.txt");
    write_utf8(&file_path, "line-1\nline-2\n").expect("写入 demo.txt 失败");
    Command::new("git")
        .args(["add", "."])
        .current_dir(&workspace_root)
        .output()
        .expect("执行 git add 失败");
    Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&workspace_root)
        .output()
        .expect("执行 git commit 失败");
    write_utf8(&file_path, "line-1\nline-2-modified\n").expect("修改 demo.txt 失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let status = GitStatusTool
        .handle(json!({}), &context)
        .await
        .expect("git_status 执行失败");
    assert!(status.contains("demo.txt"));

    let diff = GitDiffTool
        .handle(json!({}), &context)
        .await
        .expect("git_diff 执行失败");
    assert!(diff.contains("line-2-modified"));

    let log = GitLogTool
        .handle(json!({ "limit": 1 }), &context)
        .await
        .expect("git_log 执行失败");
    assert!(log.contains("init"));

    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&workspace_root)
        .output()
        .expect("读取 HEAD 失败");
    let head_id = String::from_utf8_lossy(&head.stdout).trim().to_string();

    let show = GitShowTool
        .handle(json!({ "object": head_id }), &context)
        .await
        .expect("git_show 执行失败");
    assert!(show.contains("init"));

    let blame = GitBlameTool
        .handle(
            json!({
                "path": "demo.txt",
                "start_line": 1,
                "end_line": 1
            }),
            &context,
        )
        .await
        .expect("git_blame 执行失败");
    assert!(blame.contains("line-1"));
}
