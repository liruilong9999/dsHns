//! 提示词装配器测试。
//!
//! 这些测试覆盖首次前缀拼接顺序、缺失资源告警与上下文过滤规则。

use dshns_agent::domain::workspace_session::MessageRecord;
use dshns_agent::infra::prompting::{PromptAssembler, PromptAssemblerConfig, PromptAssemblyInput};
use dshns_agent::infra::tool_system::{ToolDispatcher, ToolRuntimeConfig};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn 应按文档顺序拼接首次前缀并过滤命令审计消息() {
    let workspace_root = create_temp_directory("prompt-assembler-workspace");
    let global_agents_path = workspace_root.join("global-AGENTS.md");
    let workspace_agents_path = workspace_root.join("AGENTS.md");
    let skill_root = workspace_root.join("skills");
    fs::create_dir_all(&skill_root).expect("创建技能目录失败");

    fs::write(&global_agents_path, "全局约束内容").expect("写入全局 AGENTS 失败");
    fs::write(&workspace_agents_path, "工作区约束内容").expect("写入工作区 AGENTS 失败");
    create_skill(&skill_root, "skill-a", "Skill A", "技能 A 描述");
    create_skill(&skill_root, "skill-b", "Skill B", "技能 B 描述");

    let dispatcher = ToolDispatcher::new(ToolRuntimeConfig::new(
        workspace_root.clone(),
        skill_root.clone(),
    ));
    let assembler = PromptAssembler::new(PromptAssemblerConfig {
        global_agents_path: Some(global_agents_path),
        workspace_root_path: workspace_root,
        skill_root_path: skill_root,
        system_prompt: "系统提示词".to_string(),
    });

    let messages = vec![
        MessageRecord {
            message_id: "MSG-0001".to_string(),
            session_id: "SES-0001".to_string(),
            agent_id: None,
            round_id: "ROUND-0001".to_string(),
            sequence_no: 1,
            role: "assistant".to_string(),
            content: "历史回答内容".to_string(),
            content_type: "plain".to_string(),
            token_estimate: 0,
            include_in_context: true,
            is_compressed_source: false,
            created_at: "2026-06-06T00:00:00Z".to_string(),
        },
        MessageRecord {
            message_id: "MSG-0002".to_string(),
            session_id: "SES-0001".to_string(),
            agent_id: None,
            round_id: "ROUND-0001".to_string(),
            sequence_no: 2,
            role: "system".to_string(),
            content: "/sessions".to_string(),
            content_type: "command_audit".to_string(),
            token_estimate: 0,
            include_in_context: false,
            is_compressed_source: false,
            created_at: "2026-06-06T00:00:01Z".to_string(),
        },
    ];

    let result = assembler
        .assemble(
            &dispatcher.registry(),
            PromptAssemblyInput {
                messages: &messages,
                current_user_input: "当前用户输入",
                compression_summary: Some("压缩摘要内容"),
                context_limit: 256_000,
                expected_output_tokens: 1_024,
            },
        )
        .expect("装配提示词失败");

    assert!(result.warnings.is_empty());
    assert_order(
        &result.prompt,
        &[
            "全局约束内容",
            "工作区约束内容",
            "系统提示词",
            "Skill A",
            "Skill B",
            "压缩摘要内容",
            "历史回答内容",
            "当前用户输入",
        ],
    );
    assert!(!result.prompt.contains("/sessions"));
    assert!(result.estimated_tokens > 0);
    assert!(!result.requires_compression);
}

#[test]
fn 缺失_agents_或技能元信息时应输出中文告警并继续初始化() {
    let workspace_root = create_temp_directory("prompt-warning-workspace");
    let dispatcher = ToolDispatcher::new(ToolRuntimeConfig::new(
        workspace_root.clone(),
        workspace_root.join("missing-skills"),
    ));
    let assembler = PromptAssembler::new(PromptAssemblerConfig {
        global_agents_path: Some(workspace_root.join("missing-global-AGENTS.md")),
        workspace_root_path: workspace_root,
        skill_root_path: PathBuf::from("D:\\non-exists-skill-root"),
        system_prompt: "系统提示词".to_string(),
    });

    let result = assembler
        .assemble(
            &dispatcher.registry(),
            PromptAssemblyInput {
                messages: &[],
                current_user_input: "用户输入",
                compression_summary: None,
                context_limit: 256_000,
                expected_output_tokens: 1_024,
            },
        )
        .expect("装配提示词失败");

    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("全局 AGENTS.md 读取失败"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("工作区 AGENTS.md 读取失败"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("Skill 元信息列表读取失败"))
    );
    assert!(result.prompt.contains("系统提示词"));
    assert!(result.prompt.contains("用户输入"));
}

#[test]
fn 上下文估算达到阈值时应标记需要压缩() {
    let workspace_root = create_temp_directory("prompt-compress-workspace");
    let dispatcher = ToolDispatcher::new(ToolRuntimeConfig::new(
        workspace_root.clone(),
        workspace_root.join("skills"),
    ));
    let assembler = PromptAssembler::new(PromptAssemblerConfig {
        global_agents_path: None,
        workspace_root_path: workspace_root,
        skill_root_path: PathBuf::from("D:\\empty-skills"),
        system_prompt: "系统提示词".repeat(200),
    });

    let messages = vec![MessageRecord {
        message_id: "MSG-1000".to_string(),
        session_id: "SES-0001".to_string(),
        agent_id: None,
        round_id: "ROUND-0001".to_string(),
        sequence_no: 1,
        role: "assistant".to_string(),
        content: "很长的历史消息".repeat(800),
        content_type: "plain".to_string(),
        token_estimate: 0,
        include_in_context: true,
        is_compressed_source: false,
        created_at: "2026-06-06T00:00:00Z".to_string(),
    }];

    let result = assembler
        .assemble(
            &dispatcher.registry(),
            PromptAssemblyInput {
                messages: &messages,
                current_user_input: "继续处理当前请求",
                compression_summary: None,
                context_limit: 2_000,
                expected_output_tokens: 512,
            },
        )
        .expect("装配提示词失败");

    assert!(result.requires_compression);
}

fn assert_order(text: &str, expected_segments: &[&str]) {
    let mut last_index = 0usize;
    for segment in expected_segments {
        let current_index = text
            .find(segment)
            .unwrap_or_else(|| panic!("未找到片段：{segment}"));
        assert!(current_index >= last_index, "片段顺序错误：{segment}");
        last_index = current_index;
    }
}

fn create_skill(
    skill_root: &std::path::Path,
    directory_name: &str,
    skill_name: &str,
    description: &str,
) {
    let skill_directory = skill_root.join(directory_name);
    fs::create_dir_all(&skill_directory).expect("创建技能目录失败");
    let content =
        format!("---\nname: {skill_name}\ndescription: {description}\n---\n\n# {skill_name}\n");
    fs::write(skill_directory.join("SKILL.md"), content).expect("写入技能文件失败");
}

fn create_temp_directory(prefix: &str) -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于纪元")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("dshns-{prefix}-{unique_suffix}"));
    fs::create_dir_all(&path).expect("创建临时目录失败");
    path
}
