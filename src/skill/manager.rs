//! Skill 扫描与加载逻辑。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::utils::fs::read_optional_utf8;

/// Skill 简要信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDescriptor {
    /// Skill 名称。
    pub name: String,
    /// Skill 描述。
    pub description: String,
    /// Skill 文件路径。
    pub path: PathBuf,
}

/// Skill 管理器。
#[derive(Debug, Clone)]
pub struct SkillManager {
    /// Skill 根目录列表。
    roots: Vec<PathBuf>,
}

impl SkillManager {
    /// 创建 Skill 管理器。
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// 扫描全部可用 Skill。
    pub fn scan_skills(&self) -> Result<Vec<SkillDescriptor>> {
        let mut skills = Vec::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }

            for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
                if !entry.file_type().is_file() || entry.file_name() != "SKILL.md" {
                    continue;
                }

                if let Some(content) = read_optional_utf8(entry.path())? {
                    skills.push(self.parse_skill_descriptor(entry.path(), &content));
                }
            }
        }

        skills.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(skills)
    }

    /// 按名称或路径加载 Skill 正文。
    pub fn load_skill(&self, identifier: &str) -> Result<String> {
        let path = Path::new(identifier);
        if path.exists() {
            return read_optional_utf8(path)?
                .ok_or_else(|| anyhow!("Skill 文件不存在：{}", path.display()));
        }

        for skill in self.scan_skills()? {
            if skill.name == identifier {
                return read_optional_utf8(&skill.path)?
                    .ok_or_else(|| anyhow!("Skill 文件不存在：{}", skill.path.display()));
            }
        }

        Err(anyhow!("未找到 Skill：{}", identifier))
    }

    /// 将扫描到的 Skill 清单格式化为提示词摘要。
    pub fn render_skill_summary(&self) -> Result<String> {
        let skills = self.scan_skills()?;
        if skills.is_empty() {
            return Ok("当前未扫描到可用 Skill。".to_string());
        }

        let rendered = skills
            .iter()
            .map(|skill| {
                format!(
                    "- {}：{}（{}）",
                    skill.name,
                    skill.description,
                    skill.path.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(rendered)
    }

    /// 从 Skill 正文中提取简要信息。
    fn parse_skill_descriptor(&self, path: &Path, content: &str) -> SkillDescriptor {
        let mut name = path
            .parent()
            .and_then(|value| value.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("unknown-skill")
            .to_string();
        let mut description = "未提供描述".to_string();

        for line in content.lines().take(20) {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("name:") {
                name = value.trim().to_string();
            }
            if let Some(value) = trimmed.strip_prefix("description:") {
                description = value.trim().to_string();
            }
        }

        SkillDescriptor {
            name,
            description,
            path: path.to_path_buf(),
        }
    }
}
