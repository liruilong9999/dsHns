//! 辅助诊断类工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{
    DiagnosticsTool, ProjectMapTool, ReviewTool, RunTestsTool, ValidateDataTool,
};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::{ensure_directory, write_utf8};
use serde_json::json;
use std::process::Command;
use uuid::Uuid;

#[tokio::test]
async fn should_run_utility_tools() {
    let workspace_root = PathBuf::from(format!(
        "target/test_utility_tools_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    let src_dir = workspace_root.join("src");
    ensure_directory(&session_dir).expect("创建会话目录失败");
    ensure_directory(&src_dir).expect("创建源码目录失败");
    write_utf8(&src_dir.join("demo.rs"), "fn main() {}\n").expect("写入 demo.rs 失败");

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

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let diagnostics = DiagnosticsTool
        .handle(json!({}), &context)
        .await
        .expect("diagnostics 执行失败");
    assert!(diagnostics.contains("workspace_root"));

    let project_map = ProjectMapTool
        .handle(
            json!({
                "root_path": "src",
                "max_depth": 3,
                "max_entries": 20
            }),
            &context,
        )
        .await
        .expect("project_map 执行失败");
    assert!(project_map.contains("demo.rs"));

    let validate = ValidateDataTool
        .handle(
            json!({
                "data": { "name": "demo" },
                "required_fields": ["name", "version"]
            }),
            &context,
        )
        .await
        .expect("validate_data 执行失败");
    assert!(validate.contains("missing_fields"));
    assert!(validate.contains("version"));

    let tests_result = RunTestsTool
        .handle(
            json!({
                "command": "Write-Output 'tests ok'",
                "timeout_ms": 10000
            }),
            &context,
        )
        .await
        .expect("run_tests 执行失败");
    assert!(tests_result.contains("tests ok"));

    let review = ReviewTool
        .handle(json!({}), &context)
        .await
        .expect("review 执行失败");
    assert!(review.contains("Git 状态摘要"));
}
