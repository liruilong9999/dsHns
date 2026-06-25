use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub subagent: SubAgentConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub mode: ModeConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api: ApiConfig::default(),
            agent: AgentConfig::default(),
            subagent: SubAgentConfig::default(),
            context: ContextConfig::default(),
            mode: ModeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens_per_request: u32,
    #[serde(default = "default_timeout")]
    pub request_timeout_secs: u64,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            temperature: 0.0,
            max_tokens_per_request: default_max_tokens(),
            request_timeout_secs: default_timeout(),
        }
    }
}

fn default_model() -> String { "deepseek-v4-flash".into() }
fn default_max_tokens() -> u32 { 8192 }
fn default_timeout() -> u64 { 120 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_tool_timeout_secs")]
    pub tool_timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_tool_rounds(),
            tool_timeout_secs: default_tool_timeout_secs(),
        }
    }
}

fn default_max_tool_rounds() -> u32 { 25 }
fn default_tool_timeout_secs() -> u64 { 60 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentConfig {
    #[serde(default = "default_sub_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_sub_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_sub_msgs")]
    pub inherit_mode_max_messages: usize,
}

impl Default for SubAgentConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_sub_rounds(),
            timeout_secs: default_sub_timeout(),
            inherit_mode_max_messages: default_sub_msgs(),
        }
    }
}

fn default_sub_rounds() -> u32 { 10 }
fn default_sub_timeout() -> u64 { 300 }
fn default_sub_msgs() -> usize { 20 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_window")]
    pub max_window_tokens: usize,
    #[serde(default = "default_threshold")]
    pub compression_threshold: f64,
    #[serde(default = "default_result_tokens")]
    pub max_tool_result_tokens: usize,
    #[serde(default = "default_reserve")]
    pub reserve_tokens: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_window_tokens: default_window(),
            compression_threshold: default_threshold(),
            max_tool_result_tokens: default_result_tokens(),
            reserve_tokens: default_reserve(),
        }
    }
}

fn default_window() -> usize { 131072 }
fn default_threshold() -> f64 { 0.75 }
fn default_result_tokens() -> usize { 8000 }
fn default_reserve() -> usize { 4096 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    #[serde(default = "default_mode_val")]
    pub default: String,
}

impl Default for ModeConfig {
    fn default() -> Self { Self { default: default_mode_val() } }
}

fn default_mode_val() -> String { "auto".into() }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalMode {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "confirm")]
    Confirm,
    #[serde(rename = "paranoid")]
    Paranoid,
}

impl ApprovalMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "confirm" => Self::Confirm,
            "paranoid" => Self::Paranoid,
            _ => Self::Auto,
        }
    }
}
