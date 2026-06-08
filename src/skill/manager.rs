//! Skill 扫描与加载逻辑。
use std::fs;
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
    /// 单个 Skill 文件允许读取的最大字节数。
    max_skill_file_bytes: usize,
}

impl SkillManager {
    /// 默认的 Skill 文件大小限制。
    pub const DEFAULT_MAX_SKILL_FILE_BYTES: usize = 65_536;

    /// 创建 Skill 管理器，并使用默认大小限制。
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self::with_limits(roots, Self::DEFAULT_MAX_SKILL_FILE_BYTES)
    }

    /// 创建 Skill 管理器，并显式指定大小限制。
    pub fn with_limits(roots: Vec<PathBuf>, max_skill_file_bytes: usize) -> Self {
        Self {
            roots,
            max_skill_file_bytes,
        }
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

                let Ok(content) = self.read_skill_file(entry.path()) else {
                    // 扫描阶段跳过无效 Skill，避免污染提示词摘要。
                    continue;
                };
                skills.push(self.parse_skill_descriptor(entry.path(), &content));
            }
        }

        skills.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(skills)
    }

    /// 按名称或路径加载 Skill 正文。
    pub fn load_skill(&self, identifier: &str) -> Result<String> {
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Err(anyhow!("Skill 标识不能为空"));
        }

        if let Some(path) = self.resolve_skill_path(identifier) {
            return self.read_skill_file(&path);
        }

        for skill in self.scan_skills()? {
            if skill.name == identifier {
                return self.read_skill_file(&skill.path);
            }
        }

        Err(anyhow!("未找到目标 Skill：{}", identifier))
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

    /// 将名称或路径解析为真实 Skill 文件路径。
    fn resolve_skill_path(&self, identifier: &str) -> Option<PathBuf> {
        let raw_path = Path::new(identifier);
        let mut candidates = Vec::new();

        if raw_path.is_absolute() {
            candidates.push(raw_path.to_path_buf());
        } else {
            candidates.push(raw_path.to_path_buf());
            for root in &self.roots {
                candidates.push(root.join(raw_path));
            }
        }

        for candidate in candidates {
            let normalized = self.normalize_skill_path(&candidate);
            if normalized.exists() && normalized.is_file() {
                return Some(normalized);
            }
        }

        None
    }

    /// 规范化 Skill 路径，只允许读取 SKILL.md 或其所在目录。
    fn normalize_skill_path(&self, path: &Path) -> PathBuf {
        if path.is_dir() {
            return path.join("SKILL.md");
        }

        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("SKILL.md"))
        {
            return path.to_path_buf();
        }

        path.to_path_buf()
    }

    /// 读取并校验 Skill 文件大小与名称约束。
    fn read_skill_file(&self, path: &Path) -> Result<String> {
        let file_name = path.file_name().and_then(|value| value.to_str());
        if file_name != Some("SKILL.md") {
            return Err(anyhow!(
                "Skill 路径必须指向 SKILL.md 或其所在目录：{}",
                path.display()
            ));
        }

        let metadata = fs::metadata(path).map_err(|error| {
            anyhow!(
                "读取 Skill 文件元数据失败：{}，原因：{}",
                path.display(),
                error
            )
        })?;
        if metadata.len() > self.max_skill_file_bytes as u64 {
            return Err(anyhow!(
                "Skill 文件超过大小限制：{}，限制 {} 字节",
                path.display(),
                self.max_skill_file_bytes
            ));
        }

        read_optional_utf8(path)?.ok_or_else(|| anyhow!("Skill 文件不存在：{}", path.display()))
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

#[cfg(test)]
mod tests {
    //! Skill 管理器单元测试。
    use std::path::PathBuf;

    use uuid::Uuid;

    use crate::utils::fs::{ensure_directory, write_utf8};

    use super::SkillManager;

    /// 验证按名称、绝对路径、相对路径和目录路径都可以加载 Skill。
    #[test]
    fn should_load_skill_by_name_and_paths() {
        let root = PathBuf::from(format!("target/test_skill_manager_{}", Uuid::new_v4()));
        let skill_dir = root.join("skills").join("demo");
        let skill_file = skill_dir.join("SKILL.md");
        ensure_directory(&skill_dir).expect("创建 Skill 目录失败");
        write_utf8(
            &skill_file,
            "---\nname: demo-skill\ndescription: 测试技能\n---\n# Demo Skill\n正文",
        )
        .expect("写入 Skill 文件失败");

        let manager = SkillManager::new(vec![root.join("skills")]);
        let absolute = skill_file.canonicalize().expect("解析 Skill 绝对路径失败");
        let relative_file = skill_file.to_string_lossy().to_string();
        let relative_dir = skill_dir.to_string_lossy().to_string();

        assert!(manager
            .load_skill("demo-skill")
            .expect("按名称加载 Skill 失败")
            .contains("Demo Skill"));
        assert!(manager
            .load_skill(absolute.to_string_lossy().as_ref())
            .expect("按绝对路径加载 Skill 失败")
            .contains("Demo Skill"));
        assert!(manager
            .load_skill(&relative_file)
            .expect("按相对文件路径加载 Skill 失败")
            .contains("Demo Skill"));
        assert!(manager
            .load_skill(&relative_dir)
            .expect("按目录路径加载 Skill 失败")
            .contains("Demo Skill"));
    }

    /// 验证超限 Skill 会被扫描阶段跳过，且显式加载时返回中文错误。
    #[test]
    fn should_skip_oversized_skill_and_return_chinese_error() {
        let root = PathBuf::from(format!(
            "target/test_skill_manager_limit_{}",
            Uuid::new_v4()
        ));
        let skill_dir = root.join("skills").join("oversized");
        let skill_file = skill_dir.join("SKILL.md");
        ensure_directory(&skill_dir).expect("创建超限 Skill 目录失败");
        write_utf8(
            &skill_file,
            "---\nname: huge-skill\ndescription: 超限技能\n---\n这是一段明显超过限制的正文内容。",
        )
        .expect("写入超限 Skill 文件失败");

        let manager = SkillManager::with_limits(vec![root.join("skills")], 16);
        let scanned = manager.scan_skills().expect("扫描 Skill 失败");
        assert!(scanned.is_empty());

        let error = manager
            .load_skill(skill_file.to_string_lossy().as_ref())
            .expect_err("超限 Skill 应返回错误");
        assert!(error.to_string().contains("超过大小限制"));
    }
}
