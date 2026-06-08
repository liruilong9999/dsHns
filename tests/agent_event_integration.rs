//! 子 Agent 事件集成测试。
use std::path::PathBuf;

use dshns::ipc::bus::EventBus;
use dshns::ipc::events::EventType;
use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{AgentCloseTool, AgentEvalTool, AgentOpenTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::ensure_directory;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_emit_agent_status_events_for_agent_tools() {
    let workspace_root = PathBuf::from(format!(
        "target/test_agent_event_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session-1");
    ensure_directory(&workspace_root).expect("创建工作区目录失败");
    ensure_directory(&session_dir).expect("创建会话目录失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir: session_dir.clone(),
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    let open_tool = AgentOpenTool;
    let open_result = open_tool
        .handle(
            json!({
                "mode": "isolate",
                "inherit_context": false,
                "allowed_paths": ["src"],
                "task_spec": { "task": "demo" }
            }),
            &context,
        )
        .await
        .expect("创建子 Agent 失败");
    let open_json: serde_json::Value =
        serde_json::from_str(&open_result).expect("解析 agent_open 结果失败");
    let agent_id = open_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("缺少 agent id")
        .to_string();

    let eval_tool = AgentEvalTool;
    eval_tool
        .handle(
            json!({
                "agent_id": agent_id,
                "input": { "message": "run" }
            }),
            &context,
        )
        .await
        .expect("执行子 Agent 失败");

    let close_tool = AgentCloseTool;
    close_tool
        .handle(json!({ "agent_id": agent_id }), &context)
        .await
        .expect("关闭子 Agent 失败");

    let events = EventBus::new(session_dir)
        .list_events()
        .expect("读取子 Agent 事件失败");
    let statuses = events
        .iter()
        .filter(|event| matches!(event.event_type, EventType::AgentStatusChanged))
        .filter_map(|event| event.payload.get("status").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();

    assert!(statuses.contains(&"open"));
    assert!(statuses.contains(&"running"));
    assert!(statuses.contains(&"done"));
    assert!(statuses.contains(&"closed"));
}
