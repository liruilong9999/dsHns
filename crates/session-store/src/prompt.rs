use std::path::PathBuf;
use dshns_core::error::DshnsError;

const DEFAULT_AGENTS_MD: &str = r#"## 环境
- 操作系统：Windows
- Shell：PowerShell（powershell.exe -NoProfile）
- 编码：UTF-8（代码页 65001）
- 文件写入编码：UTF-8 无 BOM

## 安全限制（不可违反）
- 禁止使用 Remove-Item -Recurse -Force、del /f /s 等递归强制删除
- 需要删除文件时，必须逐个指定文件路径
- 禁止使用 runas 等提权命令
- 禁止对系统目录进行写操作
"#;

pub struct PromptLoader;

impl PromptLoader {
    pub fn load(working_dir: &std::path::Path) -> Result<String, DshnsError> {
        let mut parts = vec![Self::load_or_create_global()?];
        let local = working_dir.join("AGENTS.md");
        if local.exists() {
            parts.push(std::fs::read_to_string(&local)?);
        }
        Ok(parts.join("\n\n"))
    }

    fn load_or_create_global() -> Result<String, DshnsError> {
        let home = home_dir()?;
        let dir = home.join(".dsHns_rs");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("AGENTS.md");
        if !path.exists() {
            std::fs::write(&path, DEFAULT_AGENTS_MD)?;
            eprintln!("已创建默认全局提示词: {}", path.display());
        }
        Ok(std::fs::read_to_string(&path)?)
    }
}

fn home_dir() -> Result<PathBuf, DshnsError> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from).map_err(|_| DshnsError::Config("无法获取 HOME".into()))
}
