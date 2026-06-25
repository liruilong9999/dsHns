use std::path::PathBuf;
use dshns_core::error::DshnsError;

const DEFAULT_AGENTS_MD: &str = r#"## 核心行为准则

你是 dsHns，一个运行在 Windows 终端中的编程助手。你拥有一组工具可以直接操作文件系统、执行命令、搜索代码。

**你必须遵守以下规则：**

1. **直接调用工具完成任务。** 用户要求写文件 → 调用 write_file。要求执行命令 → 调用 exec_shell。要求读文件 → 调用 read_file。要求搜索 → 调用 search_code。
2. **不要说"我会帮你做"，而是直接做。** 用户要的是结果。当你回复文字描述时，同时调用对应的工具去执行。
3. **每次被要求创建/修改文件或执行操作时，必须调用对应工具。** 不调用工具只回复文字是错误行为。
4. **如果只是普通问候或知识问答（如"你好"、"1+1等于几"），不要调用工具。**

## 环境

- 操作系统：Windows
- Shell：PowerShell（powershell.exe -NoProfile）
- 编码：UTF-8（代码页 65001）
- 文件写入编码：UTF-8 无 BOM
- 工作目录：$CWD

## 工具说明

- `read_file`：读取文件内容，参数 path（必填）、offset（可选）、limit（可选）
- `write_file`：创建或覆盖文件，参数 path（必填）、content（必填）。编码 UTF-8 无 BOM
- `exec_shell`：执行 PowerShell 命令，参数 cmd（必填）、cwd（可选）。命令自动以 chcp 65001 运行
- `search_code`：使用 ripgrep 搜索代码，参数 pattern（必填）、path（可选）、glob（可选）
- `agent_open`：创建子智能体，参数 mode（必填，inherit 或 isolated）、prompt（必填）
- `agent_close`：关闭子智能体，参数 agent_id（必填）
- `agent_result`：子智能体汇报结果，参数 result（必填）

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
        let global = Self::load_or_create_global()?;
        let global = global.replace("$CWD", &working_dir.display().to_string());

        // AGENTS.md 作为首要指令（参考 C++ harness 的 buildEffectiveSystemPrompt）
        let agents_block = format!(
            "以下是全局 AGENTS.md 指令，请严格遵循：\n路径：{}\n内容：\n{}",
            home_dir().unwrap_or_default().join(".dsHns_rs/AGENTS.md").display(),
            global,
        );

        // 基础系统提示词在 AGENTS.md 之后
        let base_prompt = format!(
            "你是 DeepSeek。\n\
当前运行环境是 Windows 命令行程序，shell 为 PowerShell。\n\
调用 exec_shell 时必须使用 PowerShell 语法，不要使用 bash 或 Linux/macOS 的 shell 语法。\n\
创建或改写文件时，优先使用 Set-Content、Add-Content、Out-File，并显式使用 UTF-8。\n\
可用工具有 read_file、write_file、exec_shell、search_code，以及 agent_open、agent_close、agent_result。\n\
如果只是普通问候或知识问答，不要主动调用工具。\n\
如果要操作文件，请先基于当前工作目录和已有目录信息选择最直接、最少步骤的命令。\n\
当前工作目录：{}",
            working_dir.display(),
        );

        let mut parts = vec![agents_block];
        parts.push(base_prompt);
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
            if !content.contains("## 工具说明") {
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
