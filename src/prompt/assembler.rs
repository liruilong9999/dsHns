//! 提示词组装器实现。

use std::path::PathBuf;

use anyhow::Result;
use directories::BaseDirs;

use crate::skill::manager::SkillManager;
use crate::utils::fs::read_optional_utf8;

/// 提示词组装器。
#[derive(Clone)]
pub struct PromptAssembler {
    /// 工作区路径。
    workspace_root: PathBuf,
    /// Skill 管理器。
    skill_manager: SkillManager,
}

impl PromptAssembler {
    /// 创建提示词组装器。
    pub fn new(workspace_root: PathBuf, skill_manager: SkillManager) -> Self {
        Self {
            workspace_root,
            skill_manager,
        }
    }

    /// 组装当前会话使用的系统提示词。
    pub fn assemble(&self) -> Result<String> {
        let global_agents =
            BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex").join("AGENTS.md"));
        let workspace_agents = self.workspace_root.join("AGENTS.md");

        let global_content = global_agents
            .as_ref()
            .map(|path| read_optional_utf8(path))
            .transpose()?
            .flatten()
            .unwrap_or_else(|| "未找到全局 AGENTS.md。".to_string());
        let workspace_content = read_optional_utf8(&workspace_agents)?
            .unwrap_or_else(|| "未找到工作区 AGENTS.md。".to_string());
        let skill_summary = self.skill_manager.render_skill_summary()?;

        Ok(format!(
            "## 全局约束\n{}\n\n## 工作区约束\n{}\n\n## 可用 Skill 摘要\n{}\n",
            global_content.trim(),
            workspace_content.trim(),
            skill_summary.trim()
        ))
    }
}
