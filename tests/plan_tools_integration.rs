//! 计划工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::PlanWriteTool;
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::read_optional_utf8;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_manage_json_plan_files() {
    let workspace_root = PathBuf::from(format!(
        "target/test_plan_tools_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    let context = ToolExecutionContext {
        workspace_root,
        session_dir: session_dir.clone(),
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let tool = PlanWriteTool;
    tool.handle(
        json!({
            "plan_type": "update_plan",
            "operation": "write",
            "content": { "step": "init" }
        }),
        &context,
    )
    .await
    .expect("plan_write write 执行失败");

    tool.handle(
        json!({
            "plan_type": "checklist",
            "operation": "append",
            "content": { "title": "item-1" }
        }),
        &context,
    )
    .await
    .expect("plan_write append 执行失败");

    tool.handle(
        json!({
            "plan_type": "update_plan",
            "operation": "update",
            "content": { "status": "done" }
        }),
        &context,
    )
    .await
    .expect("plan_write update 执行失败");

    let listed = tool
        .handle(
            json!({
                "plan_type": "update_plan",
                "operation": "list"
            }),
            &context,
        )
        .await
        .expect("plan_write list 执行失败");
    assert!(listed.contains("update_plan.json"));
    assert!(listed.contains("checklist.json"));

    let update_plan = read_optional_utf8(
        &session_dir
            .join(".tools")
            .join("plan")
            .join("update_plan.json"),
    )
    .expect("读取 update_plan.json 失败")
    .expect("update_plan.json 不存在");
    let checklist = read_optional_utf8(
        &session_dir
            .join(".tools")
            .join("plan")
            .join("checklist.json"),
    )
    .expect("读取 checklist.json 失败")
    .expect("checklist.json 不存在");

    assert!(update_plan.contains("\"step\": \"init\""));
    assert!(update_plan.contains("\"status\": \"done\""));
    assert!(checklist.contains("item-1"));
}
