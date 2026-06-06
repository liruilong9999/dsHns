//! 技能目录扫描与元信息解析。
//!
//! 该模块参考外部资料中“技能系统与插件架构”的思路，统一沉淀技能的发现、
//! 元信息提取与按名称加载能力，作为提示装配和 `load_skill` 工具的共享基础。

use std::fs;
use std::path::{Path, PathBuf};

/// 技能元信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    /// 技能名称。
    pub name: String,
    /// 技能描述。
    pub description: String,
    /// 技能目录名。
    pub directory_name: String,
    /// 技能文件路径。
    pub path: PathBuf,
}

/// 技能目录扫描器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCatalog {
    /// 技能根目录。
    root: PathBuf,
}

impl SkillCatalog {
    /// 使用技能根目录构造扫描器。
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// 判断技能根目录是否存在。
    pub fn exists(&self) -> bool {
        self.root.exists()
    }

    /// 返回技能根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 列出全部技能元信息，按技能名称排序。
    pub fn list_metadata(&self) -> Vec<SkillMetadata> {
        let mut items = Vec::new();
        self.collect_skill_files_recursive(&self.root, &mut items);
        items.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.directory_name.cmp(&right.directory_name))
        });
        items
    }

    /// 按技能名或目录名查找技能文件。
    pub fn find_skill_file(&self, skill_name: &str) -> Option<PathBuf> {
        let target = skill_name.trim();
        if target.is_empty() {
            return None;
        }

        let direct_path = self.root.join(target).join("SKILL.md");
        if direct_path.exists() {
            return Some(direct_path);
        }

        self.list_metadata().into_iter().find_map(|metadata| {
            if metadata.name == target || metadata.directory_name == target {
                Some(metadata.path)
            } else {
                None
            }
        })
    }

    /// 递归收集技能文件。
    fn collect_skill_files_recursive(&self, root: &Path, items: &mut Vec<SkillMetadata>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() {
                    if let Some(metadata) = parse_skill_metadata(&skill_file) {
                        items.push(metadata);
                    }
                }
                self.collect_skill_files_recursive(&path, items);
            }
        }
    }
}

/// 解析单个技能文件的元信息。
fn parse_skill_metadata(path: &Path) -> Option<SkillMetadata> {
    let content = fs::read_to_string(path).ok()?;
    let directory_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown-skill".to_string());

    let (name, description) = parse_frontmatter_name_and_description(&content);
    Some(SkillMetadata {
        name: name.unwrap_or_else(|| directory_name.clone()),
        description: description.unwrap_or_else(|| "无描述".to_string()),
        directory_name,
        path: path.to_path_buf(),
    })
}

/// 从技能文件中解析 `name` 和 `description`。
fn parse_frontmatter_name_and_description(content: &str) -> (Option<String>, Option<String>) {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return parse_fallback_name_and_description(content);
    }

    let mut name = None;
    let mut description = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(value.trim().to_string());
        }
    }

    (name, description)
}

/// 当前罕见情况下的兼容解析。
fn parse_fallback_name_and_description(content: &str) -> (Option<String>, Option<String>) {
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
    (name, description)
}
