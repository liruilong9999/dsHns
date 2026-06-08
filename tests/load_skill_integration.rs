//! load_skill 工具集成测试。
use std::path::PathBuf;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::LoadSkillTool;
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::{ensure_directory, write_utf8};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_load_skill_tool_by_name_and_path() {
    let workspace_root = PathBuf::from(format!(
        "target/test_load_skill_workspace_{}",
        Uuid::new_v4()
    ));
    let session_dir = workspace_root.join("session");
    let skill_dir = workspace_root.join("skills").join("demo");
    let skill_file = skill_dir.join("SKILL.md");

    ensure_directory(&session_dir).expect("创建会话目录失败");
    ensure_directory(&skill_dir).expect("创建 Skill 目录失败");
    write_utf8(
        &skill_file,
        "---\nname: demo-skill\ndescription: 集成测试技能\n---\n# Demo Skill\n正文内容",
    )
    .expect("写入 Skill 文件失败");

    let context = ToolExecutionContext {
        workspace_root: workspace_root.clone(),
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::with_limits(vec![workspace_root.join("skills")], 65_536),
    };
    let tool = LoadSkillTool;

    let by_name = tool
        .handle(json!({ "identifier": "demo-skill" }), &context)
        .await
        .expect("按名称调用 load_skill 失败");
    assert!(by_name.contains("Demo Skill"));

    let by_path = tool
        .handle(
            json!({ "identifier": skill_dir.to_string_lossy().to_string() }),
            &context,
        )
        .await
        .expect("按目录路径调用 load_skill 失败");
    assert!(by_path.contains("正文内容"));
}
