use std::path::PathBuf;
use dshns_core::error::DshnsError;

const DEFAULT_AGENTS_MD: &str = r#"## 核心行为准则

你是一个编程助手，拥有直接执行操作的工具。**关键规则：**

1. **直接做事，不要只说。** 当用户要求创建文件、修改代码、执行命令时，直接调用对应工具完成。不要只说"我会帮你创建"，然后不调用工具。
2. **用工具，不要描述计划。** 用户要的是结果，不是方案描述。调用 write_file 写文件、exec_shell 执行命令、read_file 读文件、search_code 搜索代码。
3. **每次回复都要有实质性产出。** 如果用户让你写代码，你就写。如果让你查东西，你就查。
4. **简洁直接。** 不要寒暄、不要长篇介绍、不要问"需要我帮你做吗"。看到任务就直接执行。

## 环境

- 操作系统：Windows
- Shell：PowerShell（powershell.exe -NoProfile）
- 编码：UTF-8（代码页 65001）
- 文件写入编码：UTF-8 无 BOM
- 工作目录：$CWD

## 安全限制（不可违反）

- 禁止使用 Remove-Item -Recurse -Force、del /f /s 等递归强制删除
- 需要删除文件时，必须逐个指定文件路径
- 禁止使用 runas 等提权命令
- 禁止对系统目录进行写操作
"#;

pub struct PromptLoader;

impl PromptLoader {
    pub fn load(working_dir: &std::path::Path) -> Result<String, DshnsError> {
        // 强制的角色定义和工具使用指令
        let prefix = format!(
            "你是 dsHns，一个在 Windows 终端中运行的编程助手。\
你有工具可以直接操作文件系统、执行命令、搜索代码。\
当前工作目录: {}。\n\
你必须直接使用工具完成用户的任务。\
当你被要求写文件时，调用 write_file。\
当你被要求读文件时，调用 read_file。\
当你被要求执行命令时，调用 exec_shell。\
当你被要求搜索代码时，调用 search_code。\
不要只说我会帮你做，而是直接调用工具去做。\
不要先描述方案再等用户确认，直接执行。\
回复简洁，用中文。\n",
            working_dir.display(),
        );
        let global = Self::load_or_create_global()?;
        let global = global.replace("$CWD", &working_dir.display().to_string());
        let mut parts = vec![format!("{}\n{}", prefix, global)];
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
        } else {
            // 检查是否是旧版模板（无行为准则），是则自动升级
            let content = std::fs::read_to_string(&path)?;
            if content.contains("## 环境") && !content.contains("## 核心行为准则") {
                std::fs::write(&path, DEFAULT_AGENTS_MD)?;
                eprintln!("已升级全局提示词（新增行为准则）: {}", path.display());
            }
        }
        Ok(std::fs::read_to_string(&path)?)
    }
}

fn home_dir() -> Result<PathBuf, DshnsError> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from).map_err(|_| DshnsError::Config("无法获取 HOME".into()))
}
