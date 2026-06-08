//! 配置定义与加载逻辑。
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use configparser::ini::Ini;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::domain::ApprovalMode;
use crate::utils::fs::ensure_directory;

/// 应用配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// 工作区根目录。
    pub workspace_root: PathBuf,
    /// 内部数据根目录。
    pub data_root: PathBuf,
    /// 会话目录根路径。
    pub sessions_root: PathBuf,
    /// SQLite 数据库路径。
    pub database_path: PathBuf,
    /// setting.ini 文件路径。
    pub setting_file: PathBuf,
    /// 默认模型名称。
    pub default_model: String,
    /// 可选模型列表。
    pub allowed_models: Vec<String>,
    /// 默认审批模式。
    pub default_approval_mode: ApprovalMode,
    /// 是否默认开启流式输出。
    pub default_stream_output: bool,
    /// 单轮最大循环次数。
    pub max_rounds: usize,
    /// 连续非法参数阈值。
    pub invalid_arg_retry_limit: usize,
    /// 单工具连续失败阈值。
    pub tool_failure_limit: usize,
    /// 单轮工具调用总上限。
    pub tool_call_limit: usize,
    /// 工具结果直接内联的最大字符数。
    pub inline_output_limit: usize,
    /// 默认 Shell 程序。
    pub shell_program: String,
    /// DeepSeek 兼容接口地址。
    pub deepseek_base_url: String,
    /// Skill 根目录列表。
    pub skill_roots: Vec<PathBuf>,
    /// 单个 Skill 文件允许读取的最大字节数。
    pub max_skill_file_bytes: usize,
    /// 是否允许网络工具。
    pub allow_network: bool,
    /// 是否允许 Shell 工具。
    pub allow_shell: bool,
    /// 是否允许写文件工具。
    pub allow_file_write: bool,
    /// 是否允许插件类工具。
    pub allow_plugin_tool: bool,
}

impl Settings {
    /// 从工作区加载配置，不存在 setting.ini 时自动回退到默认值。
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let workspace_root = workspace_root.to_path_buf();
        let data_root = workspace_root.join(".dshns");
        let sessions_root = data_root.join("sessions");
        let database_path = data_root.join("harness.db");
        let setting_file = workspace_root.join("setting.ini");
        let home_skill_root = BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".codex").join("skills"))
            .unwrap_or_else(|| workspace_root.join("skills"));

        let mut settings = Self {
            workspace_root: workspace_root.clone(),
            data_root,
            sessions_root,
            database_path,
            setting_file: setting_file.clone(),
            default_model: "deepseek-v4-flash".to_string(),
            allowed_models: vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash[1m]".to_string(),
                "deepseek-v4-pro[1m]".to_string(),
            ],
            default_approval_mode: ApprovalMode::AskUser,
            default_stream_output: true,
            max_rounds: 40,
            invalid_arg_retry_limit: 3,
            tool_failure_limit: 5,
            tool_call_limit: 20,
            inline_output_limit: 1200,
            shell_program: "powershell".to_string(),
            deepseek_base_url: "https://api.deepseek.com/chat/completions".to_string(),
            skill_roots: vec![workspace_root.join("skills"), home_skill_root],
            max_skill_file_bytes: 65_536,
            allow_network: true,
            allow_shell: false,
            allow_file_write: false,
            allow_plugin_tool: false,
        };

        if setting_file.exists() {
            let mut ini = Ini::new();
            ini.load(setting_file.to_string_lossy().as_ref())
                .map_err(|error| {
                    anyhow!(
                        "读取配置文件失败：{}，原因：{}",
                        setting_file.display(),
                        error
                    )
                })?;

            settings.default_model = ini
                .get("model", "default")
                .or_else(|| ini.get("api", "model"))
                .filter(|value| !value.is_empty())
                .unwrap_or(settings.default_model);
            settings.default_stream_output = ini
                .getbool("model", "stream_output")
                .ok()
                .flatten()
                .or_else(|| ini.getbool("cli", "stream_output").ok().flatten())
                .unwrap_or(settings.default_stream_output);
            settings.max_rounds = ini
                .getuint("limits", "max_rounds")
                .ok()
                .flatten()
                .or_else(|| ini.getuint("agent", "max_rounds").ok().flatten())
                .map(|value| value as usize)
                .unwrap_or(settings.max_rounds);
            settings.invalid_arg_retry_limit = ini
                .getuint("limits", "invalid_arg_retry_limit")
                .ok()
                .flatten()
                .map(|value| value as usize)
                .unwrap_or(settings.invalid_arg_retry_limit);
            settings.tool_failure_limit = ini
                .getuint("limits", "tool_failure_limit")
                .ok()
                .flatten()
                .map(|value| value as usize)
                .unwrap_or(settings.tool_failure_limit);
            settings.tool_call_limit = ini
                .getuint("limits", "tool_call_limit")
                .ok()
                .flatten()
                .map(|value| value as usize)
                .unwrap_or(settings.tool_call_limit);
            settings.deepseek_base_url = ini
                .get("deepseek", "base_url")
                .or_else(|| ini.get("api", "base_url"))
                .filter(|value| !value.is_empty())
                .unwrap_or(settings.deepseek_base_url);
            settings.max_skill_file_bytes = ini
                .getuint("skill", "max_skill_file_bytes")
                .ok()
                .flatten()
                .map(|value| value as usize)
                .unwrap_or(settings.max_skill_file_bytes);
            settings.default_approval_mode = parse_approval_mode(
                ini.get("approval", "mode").as_deref(),
                settings.default_approval_mode,
            );
            settings.allow_network = ini
                .getbool("approval", "allow_network")
                .ok()
                .flatten()
                .unwrap_or(settings.allow_network);
            settings.allow_shell = ini
                .getbool("approval", "allow_shell")
                .ok()
                .flatten()
                .unwrap_or(settings.allow_shell);
            settings.allow_file_write = ini
                .getbool("approval", "allow_file_write")
                .ok()
                .flatten()
                .unwrap_or(settings.allow_file_write);
            settings.allow_plugin_tool = ini
                .getbool("approval", "allow_plugin_tool")
                .ok()
                .flatten()
                .unwrap_or(settings.allow_plugin_tool);
        }

        settings.ensure_layout()?;
        Ok(settings)
    }

    /// 初始化运行所需目录结构。
    pub fn ensure_layout(&self) -> Result<()> {
        ensure_directory(&self.data_root)?;
        ensure_directory(&self.sessions_root)?;
        Ok(())
    }

    /// 判断模型是否在允许清单中。
    pub fn is_allowed_model(&self, model: &str) -> bool {
        self.allowed_models.iter().any(|item| item == model)
    }
}

fn parse_approval_mode(raw: Option<&str>, fallback: ApprovalMode) -> ApprovalMode {
    match raw.unwrap_or_default().trim() {
        "0" | "AskUser" => ApprovalMode::AskUser,
        "1" | "AutoApproveSafe" => ApprovalMode::AutoApproveSafe,
        "2" | "FullAccess" => ApprovalMode::FullAccess,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    //! 配置默认值测试。
    use std::path::Path;

    use crate::domain::ApprovalMode;
    use crate::utils::fs::write_utf8;

    use super::Settings;

    /// 验证缺省配置可以正常回退。
    #[test]
    fn should_load_default_settings_without_ini() {
        let settings = Settings::load(Path::new(".")).expect("加载默认配置失败");
        assert_eq!(settings.default_model, "deepseek-v4-flash");
        assert!(settings.is_allowed_model("deepseek-v4-pro"));
        assert_eq!(settings.max_skill_file_bytes, 65_536);
        assert!(settings.allow_network);
        assert!(!settings.allow_shell);
        assert!(!settings.allow_file_write);
    }

    /// 验证可以从 approval 分组读取能力开关。
    #[test]
    fn should_load_approval_flags_from_ini() {
        let workspace = Path::new("target/test_settings_with_approval_flags");
        write_utf8(
            &workspace.join("setting.ini"),
            "[approval]\nmode=1\nallow_network=false\nallow_shell=true\nallow_file_write=true\nallow_plugin_tool=true\n",
        )
        .expect("写入 setting.ini 失败");

        let settings = Settings::load(workspace).expect("加载带审批开关的配置失败");
        assert_eq!(settings.default_approval_mode, ApprovalMode::AutoApproveSafe);
        assert!(!settings.allow_network);
        assert!(settings.allow_shell);
        assert!(settings.allow_file_write);
        assert!(settings.allow_plugin_tool);
    }
}
