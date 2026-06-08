//! 自动化工具生命周期集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{
    AutomationCreateTool, AutomationDeleteTool, AutomationListTool, AutomationPauseTool,
    AutomationReadTool, AutomationResumeTool, AutomationRunOnceTool, AutomationUpdateTool,
};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::ensure_directory;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_manage_automation_lifecycle() {
    let workspace_root = PathBuf::from(format!(
        "target/test_automation_tools_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    ensure_directory(&session_dir).expect("创建会话目录失败");

    let context = ToolExecutionContext {
        workspace_root,
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let create_tool = AutomationCreateTool;
    let created = create_tool
        .handle(
            json!({
                "name": "demo-automation",
                "kind": "shell",
                "status": "active",
                "definition": { "command": "Write-Output 'automation-ok'" }
            }),
            &context,
        )
        .await
        .expect("automation_create 执行失败");
    let created_json: serde_json::Value =
        serde_json::from_str(&created).expect("解析 automation_create 结果失败");
    let automation_id = created_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("缺少 automation id")
        .to_string();

    let list_tool = AutomationListTool;
    let listed = list_tool
        .handle(json!({}), &context)
        .await
        .expect("automation_list 执行失败");
    assert!(listed.contains("demo-automation"));

    let read_tool = AutomationReadTool;
    let read_result = read_tool
        .handle(json!({ "automation_id": automation_id }), &context)
        .await
        .expect("automation_read 执行失败");
    assert!(read_result.contains("\"status\": \"active\""));

    let update_tool = AutomationUpdateTool;
    let updated = update_tool
        .handle(
            json!({
                "automation_id": automation_id,
                "name": "demo-automation-v2",
                "status": "paused"
            }),
            &context,
        )
        .await
        .expect("automation_update 执行失败");
    assert!(updated.contains("demo-automation-v2"));
    assert!(updated.contains("\"status\": \"paused\""));

    let pause_tool = AutomationPauseTool;
    let paused = pause_tool
        .handle(json!({ "automation_id": automation_id }), &context)
        .await
        .expect("automation_pause 执行失败");
    assert!(paused.contains("\"status\": \"paused\""));

    let resume_tool = AutomationResumeTool;
    let resumed = resume_tool
        .handle(json!({ "automation_id": automation_id }), &context)
        .await
        .expect("automation_resume 执行失败");
    assert!(resumed.contains("\"status\": \"active\""));

    let run_once_tool = AutomationRunOnceTool;
    let output = run_once_tool
        .handle(
            json!({
                "automation_id": automation_id,
                "timeout_ms": 10000
            }),
            &context,
        )
        .await
        .expect("automation_run_once 执行失败");
    assert!(output.contains("automation-ok"));

    let delete_tool = AutomationDeleteTool;
    let deleted = delete_tool
        .handle(json!({ "automation_id": automation_id }), &context)
        .await
        .expect("automation_delete 执行失败");
    assert!(deleted.contains("已删除自动化"));
}
