//! 后台 Shell 工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{
    ExecShellCancelTool, ExecShellInteractTool, ExecShellTool, ExecShellWaitTool,
};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::ensure_directory;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_run_exec_shell_chain() {
    let workspace_root = PathBuf::from(format!(
        "target/test_exec_shell_workspace_{}",
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

    let open_tool = ExecShellTool;
    let open_result = open_tool
        .handle(
            json!({
                "command": "Write-Output 'first-output'"
            }),
            &context,
        )
        .await
        .expect("exec_shell 执行失败");
    let open_json: serde_json::Value =
        serde_json::from_str(&open_result).expect("解析 exec_shell 结果失败");
    let process_id = open_json
        .get("process_id")
        .and_then(serde_json::Value::as_str)
        .expect("缺少 process_id")
        .to_string();

    let wait_tool = ExecShellWaitTool;
    let first_output = wait_tool
        .handle(
            json!({
                "process_id": process_id,
                "idle_timeout_ms": 300,
                "max_lines": 20
            }),
            &context,
        )
        .await
        .expect("exec_shell_wait 首次执行失败");
    assert!(first_output.contains("first-output"));

    let interact_tool = ExecShellInteractTool;
    interact_tool
        .handle(
            json!({
                "process_id": process_id,
                "input": "Write-Output 'second-output'"
            }),
            &context,
        )
        .await
        .expect("exec_shell_interact 执行失败");

    let second_output = wait_tool
        .handle(
            json!({
                "process_id": process_id,
                "idle_timeout_ms": 300,
                "max_lines": 20
            }),
            &context,
        )
        .await
        .expect("exec_shell_wait 二次执行失败");
    assert!(second_output.contains("second-output"));

    let cancel_tool = ExecShellCancelTool;
    let cancel_result = cancel_tool
        .handle(json!({ "process_id": process_id }), &context)
        .await
        .expect("exec_shell_cancel 执行失败");
    assert!(cancel_result.contains("cancelled"));
}
