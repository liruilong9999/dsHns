//! 提示词装配器实现。
//!
//! 本模块负责读取 `AGENTS.md`、技能元信息与会话消息，并按文档顺序拼接提示词。

use crate::domain::tool::ToolMetadata;
use crate::domain::workspace_session::MessageRecord;
use crate::infra::tool_system::ToolRegistry;
use std::fs;
use std::path::{Path, PathBuf};

/// 提示词装配器配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAssemblerConfig {
    /// 全局 `AGENTS.md` 路径。
    pub global_agents_path: Option<PathBuf>,
    /// 工作区根目录路径。
    pub workspace_root_path: PathBuf,
    /// 技能根目录路径。
    pub skill_root_path: PathBuf,
    /// 系统提示词。
    pub system_prompt: String,
}

/// 提示词装配输入。
#[derive(Debug, Clone)]
pub struct PromptAssemblyInput<'a> {
    /// 当前会话历史消息。
    pub messages: &'a [MessageRecord],
    /// 当前用户输入。
    pub current_user_input: &'a str,
    /// 可选压缩摘要。
    pub compression_summary: Option<&'a str>,
    /// 当前上下文上限。
    pub context_limit: u32,
    /// 预计输出 `Token`。
    pub expected_output_tokens: u32,
}

/// 提示词装配结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAssemblyResult {
    /// 最终装配出的提示词。
    pub prompt: String,
    /// 中文告警列表。
    pub warnings: Vec<String>,
    /// 估算输入 `Token`。
    pub estimated_tokens: u32,
    /// 是否达到压缩阈值。
    pub requires_compression: bool,
}

/// 提示词装配器。
pub struct PromptAssembler {
    /// 装配配置。
    config: PromptAssemblerConfig,
}

impl PromptAssembler {
    /// 构造提示词装配器。
    pub fn new(config: PromptAssemblerConfig) -> Self {
        Self { config }
    }

    /// 按文档顺序装配提示词。
    pub fn assemble(
        &self,
        tool_registry: &ToolRegistry,
        input: PromptAssemblyInput<'_>,
    ) -> Result<PromptAssemblyResult, String> {
        let mut warnings = Vec::new();
        let mut sections = Vec::new();

        if let Some(global_path) = &self.config.global_agents_path {
            match fs::read_to_string(global_path) {
                Ok(content) => {
                    if !content.trim().is_empty() {
                        sections.push(content);
                    }
                }
                Err(error) => warnings.push(format!(
                    "全局 AGENTS.md 读取失败，已跳过：{}，原因：{error}",
                    global_path.display()
                )),
            }
        }

        let workspace_agents_path = self.config.workspace_root_path.join("AGENTS.md");
        match fs::read_to_string(&workspace_agents_path) {
            Ok(content) => {
                if !content.trim().is_empty() {
                    sections.push(content);
                }
            }
            Err(error) => warnings.push(format!(
                "工作区 AGENTS.md 读取失败，已跳过：{}，原因：{error}",
                workspace_agents_path.display()
            )),
        }

        if !self.config.system_prompt.trim().is_empty() {
            sections.push(self.config.system_prompt.clone());
        }

        let skill_metadata_summary =
            load_skill_metadata_summary(&self.config.skill_root_path, &mut warnings);
        if !skill_metadata_summary.is_empty() {
            sections.push(skill_metadata_summary);
        }

        if let Some(summary) = input.compression_summary {
            if !summary.trim().is_empty() {
                sections.push(summary.to_string());
            }
        }

        let filtered_messages = input
            .messages
            .iter()
            .filter(|message| message.include_in_context && message.content_type != "command_audit")
            .map(format_message_for_prompt)
            .collect::<Vec<_>>();
        if !filtered_messages.is_empty() {
            sections.push(filtered_messages.join("\n"));
        }

        sections.push(format!("[用户]\n{}", input.current_user_input));

        let prompt = sections.join("\n\n");
        let estimated_tokens = estimate_tokens(&prompt);
        let tool_tokens = estimate_tool_metadata_tokens(tool_registry);
        let skill_tokens = estimate_tokens(&load_skill_metadata_summary_no_warning(
            &self.config.skill_root_path,
        ));
        let safety_margin = if input.context_limit >= 1_000_000 {
            32_768
        } else {
            8_192
        };
        let requires_compression = estimated_tokens
            + tool_tokens
            + skill_tokens
            + safety_margin
            + input.expected_output_tokens
            > input.context_limit
            || estimated_tokens + tool_tokens + skill_tokens + safety_margin
                > ((input.context_limit as f64) * 0.85) as u32;

        Ok(PromptAssemblyResult {
            prompt,
            warnings,
            estimated_tokens,
            requires_compression,
        })
    }
}

/// 读取技能元信息摘要，并在失败时写入中文告警。
fn load_skill_metadata_summary(skill_root_path: &Path, warnings: &mut Vec<String>) -> String {
    if !skill_root_path.exists() {
        warnings.push(format!(
            "Skill 元信息列表读取失败，已跳过：{} 不存在。",
            skill_root_path.display()
        ));
        return String::new();
    }

    let skill_files = collect_skill_files(skill_root_path);
    if skill_files.is_empty() {
        warnings.push(format!(
            "Skill 元信息列表读取失败，已跳过：{} 下未找到 SKILL.md。",
            skill_root_path.display()
        ));
        return String::new();
    }

    format_skill_metadata_summary(skill_files)
}

/// 不产生日志告警地读取技能元信息摘要。
fn load_skill_metadata_summary_no_warning(skill_root_path: &Path) -> String {
    if !skill_root_path.exists() {
        return String::new();
    }

    let skill_files = collect_skill_files(skill_root_path);
    if skill_files.is_empty() {
        return String::new();
    }

    format_skill_metadata_summary(skill_files)
}

/// 收集技能文件列表。
fn collect_skill_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_skill_files_recursive(root, &mut files);
    files.sort();
    files
}

/// 递归收集技能文件。
fn collect_skill_files_recursive(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files_recursive(&path, files);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            files.push(path);
        }
    }
}

/// 把技能文件列表转换成元信息摘要。
fn format_skill_metadata_summary(skill_files: Vec<PathBuf>) -> String {
    let lines = skill_files
        .into_iter()
        .filter_map(|path| parse_skill_metadata(&path))
        .map(|metadata| format!("{}：{}", metadata.name, metadata.description))
        .collect::<Vec<_>>();

    if lines.is_empty() {
        String::new()
    } else {
        format!("[Skill 列表]\n{}", lines.join("\n"))
    }
}

/// 技能元信息。
#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillMetadataSummary {
    /// 技能名。
    name: String,
    /// 技能描述。
    description: String,
}

/// 解析技能元信息。
fn parse_skill_metadata(path: &Path) -> Option<SkillMetadataSummary> {
    let content = fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut description = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(value.trim().to_string());
        }
    }

    Some(SkillMetadataSummary {
        name: name.unwrap_or_else(|| {
            path.parent()
                .and_then(|parent| parent.file_name())
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown-skill".to_string())
        }),
        description: description.unwrap_or_else(|| "无描述".to_string()),
    })
}

/// 估算工具元信息的 `Token` 数。
fn estimate_tool_metadata_tokens(tool_registry: &ToolRegistry) -> u32 {
    let text = tool_registry
        .all_metadata()
        .into_iter()
        .map(format_tool_metadata_summary)
        .collect::<Vec<_>>()
        .join("\n");
    estimate_tokens(&text)
}

/// 格式化工具元信息摘要。
fn format_tool_metadata_summary(metadata: &ToolMetadata) -> String {
    format!("{}：{}", metadata.name, metadata.description)
}

/// 格式化消息供提示词装配器使用。
fn format_message_for_prompt(message: &MessageRecord) -> String {
    format!("[{}]\n{}", message.role, message.content)
}

/// 简化的 `Token` 估算。
fn estimate_tokens(text: &str) -> u32 {
    let char_count = text.chars().count() as u32;
    (char_count / 4).max(1)
}
