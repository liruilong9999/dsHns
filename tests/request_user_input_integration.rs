//! request_user_input 工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::RequestUserInputTool;
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::ensure_directory;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_generate_structured_user_input_request() {
    let workspace_root = PathBuf::from(format!(
        "target/test_request_user_input_workspace_{}",
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

    let tool = RequestUserInputTool;
    let result = tool
        .handle(
            json!({
                "questions": [
                    {
                        "header": "模型",
                        "id": "model_choice",
                        "question": "请选择要继续使用的模型",
                        "options": [
                            { "label": "flash", "description": "响应更快" },
                            { "label": "pro", "description": "质量更高" }
                        ]
                    }
                ]
            }),
            &context,
        )
        .await
        .expect("request_user_input 执行失败");

    assert!(result.contains("\"kind\": \"request_user_input\""));
    assert!(result.contains("\"model_choice\""));
}
