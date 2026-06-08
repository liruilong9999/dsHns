//! 任务后台命令工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{TaskCreateTool, TaskShellStartTool, TaskShellWaitTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::ensure_directory;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_bind_task_to_background_shell_and_wait() {
    let workspace_root = PathBuf::from(format!(
        "target/test_task_shell_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    ensure_directory(&session_dir).expect("创建会话目录失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let create_tool = TaskCreateTool;
    let created = create_tool
        .handle(
            json!({
                "name": "bg-task",
                "command": "Write-Output 'task-shell-output'"
            }),
            &context,
        )
        .await
        .expect("task_create 执行失败");
    let created_json: serde_json::Value =
        serde_json::from_str(&created).expect("解析 task_create 结果失败");
    let task_id = created_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("缺少 task id")
        .to_string();

    let start_tool = TaskShellStartTool;
    let start_result = start_tool
        .handle(json!({ "task_id": task_id }), &context)
        .await
        .expect("task_shell_start 执行失败");
    assert!(start_result.contains("process_id"));

    let wait_tool = TaskShellWaitTool;
    let output = wait_tool
        .handle(
            json!({
                "task_id": task_id,
                "idle_timeout_ms": 300,
                "max_lines": 20
            }),
            &context,
        )
        .await
        .expect("task_shell_wait 执行失败");
    assert!(output.contains("task-shell-output"));
}
