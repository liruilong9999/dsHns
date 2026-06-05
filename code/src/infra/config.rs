//! 配置加载模块。
//!
//! 本模块负责承接文档中约定的默认模型、上下文限制、审批模式以及
//! `DEEPSEEK_API_KEY` 环境变量读取逻辑。

use std::fmt::{self, Debug, Formatter};

/// 模型访问 `key` 的环境变量名。
pub const MODEL_API_KEY_ENV_NAME: &str = "DEEPSEEK_API_KEY";

/// 环境变量读取抽象。
///
/// 通过抽象环境变量来源，测试时可以注入假数据，避免直接污染进程环境。
pub trait EnvSource {
    /// 读取指定环境变量的值。
    fn read(&self, key: &str) -> Option<String>;
}

/// 直接读取当前进程环境变量的实现。
pub struct ProcessEnvSource;

impl EnvSource for ProcessEnvSource {
    fn read(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// 会话审批模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// 询问后执行。
    Ask,
    /// 自动执行允许的工具。
    Auto,
    /// 跳过人工确认，但不跳过参数校验与边界检查。
    AllowAll,
}

/// 模型网关不可用原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelGatewayIssue {
    /// 环境变量缺失。
    MissingApiKey,
    /// 环境变量去除首尾空白后为空。
    EmptyApiKey,
}

/// 模型网关可用性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelGatewayAvailability {
    /// 模型网关可用。
    Available,
    /// 模型网关不可用，并给出具体原因。
    Unavailable(ModelGatewayIssue),
}

/// 敏感字符串包装器。
///
/// 该类型用于缓存 `DEEPSEEK_API_KEY`，并通过自定义 `Debug` 避免日志或调试输出泄漏明文。
#[derive(Clone, PartialEq, Eq)]
pub struct SensitiveString {
    /// 实际的敏感字符串内容。
    raw: String,
}

impl SensitiveString {
    /// 使用给定明文构造敏感字符串包装器。
    pub fn new(raw: String) -> Self {
        Self { raw }
    }

    /// 获取敏感值明文引用。
    ///
    /// 当前阶段仅为后续模型网关调用预留访问能力，调用方应避免直接输出。
    pub fn expose(&self) -> &str {
        &self.raw
    }
}

impl Debug for SensitiveString {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("\"[已隐藏的敏感信息]\"")
    }
}

/// 模型定义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDefinition {
    /// 模型名称。
    pub name: String,
    /// 默认上下文上限。
    pub context_limit: u32,
}

/// 模型网关配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelGatewayConfig {
    /// 模型网关当前可用性。
    availability: ModelGatewayAvailability,
    /// 模型访问 `key` 的环境变量名。
    required_env_name: &'static str,
    /// 缓存的敏感 `key`。
    api_key: Option<SensitiveString>,
}

impl ModelGatewayConfig {
    /// 获取模型网关可用性。
    pub fn availability(&self) -> &ModelGatewayAvailability {
        &self.availability
    }

    /// 获取用户可见的中文状态说明。
    pub fn user_facing_message(&self) -> &'static str {
        match self.availability() {
            ModelGatewayAvailability::Available => "模型网关可用。",
            ModelGatewayAvailability::Unavailable(ModelGatewayIssue::MissingApiKey) => {
                "环境变量 DEEPSEEK_API_KEY 缺失，模型网关不可用，请设置后重启应用。"
            }
            ModelGatewayAvailability::Unavailable(ModelGatewayIssue::EmptyApiKey) => {
                "环境变量 DEEPSEEK_API_KEY 去除首尾空白后为空，模型网关不可用，请设置有效值后重启应用。"
            }
        }
    }

    /// 获取缓存的敏感 `key`。
    pub fn api_key(&self) -> Option<&SensitiveString> {
        self.api_key.as_ref()
    }

    /// 获取必需的环境变量名。
    pub fn required_env_name(&self) -> &'static str {
        self.required_env_name
    }
}

/// 应用配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    /// 支持的模型列表。
    models: Vec<ModelDefinition>,
    /// 默认模型名。
    default_model: String,
    /// 默认审批模式。
    default_approval_mode: ApprovalMode,
    /// 模型网关配置。
    model_gateway: ModelGatewayConfig,
}

impl AppConfig {
    /// 从当前进程环境加载应用配置。
    pub fn load() -> Self {
        Self::load_from_env(&ProcessEnvSource)
    }

    /// 从指定环境变量来源加载应用配置。
    pub fn load_from_env(env_source: &dyn EnvSource) -> Self {
        let api_key_state = Self::load_api_key(env_source);

        Self {
            models: Self::default_models(),
            default_model: "deepseek-v4-flash".to_string(),
            default_approval_mode: ApprovalMode::Ask,
            model_gateway: api_key_state,
        }
    }

    /// 获取默认模型名。
    pub fn default_model_name(&self) -> &str {
        &self.default_model
    }

    /// 获取默认审批模式。
    pub fn default_approval_mode(&self) -> ApprovalMode {
        self.default_approval_mode
    }

    /// 获取支持的模型名列表。
    pub fn available_model_names(&self) -> Vec<String> {
        self.models.iter().map(|model| model.name.clone()).collect()
    }

    /// 按模型名查询默认上下文上限。
    pub fn context_limit_for_model(&self, model_name: &str) -> Option<u32> {
        self.models
            .iter()
            .find(|model| model.name == model_name)
            .map(|model| model.context_limit)
    }

    /// 获取模型网关配置。
    pub fn model_gateway(&self) -> &ModelGatewayConfig {
        &self.model_gateway
    }

    /// 构建文档约定的默认模型列表。
    fn default_models() -> Vec<ModelDefinition> {
        vec![
            ModelDefinition {
                name: "deepseek-v4-flash".to_string(),
                context_limit: 256_000,
            },
            ModelDefinition {
                name: "deepseek-v4-pro".to_string(),
                context_limit: 256_000,
            },
            ModelDefinition {
                name: "deepseek-v4-flash[1m]".to_string(),
                context_limit: 1_000_000,
            },
            ModelDefinition {
                name: "deepseek-v4-pro[1m]".to_string(),
                context_limit: 1_000_000,
            },
        ]
    }

    /// 读取并缓存模型访问 `key`。
    ///
    /// 这里严格只从 `DEEPSEEK_API_KEY` 读取，不允许回退到其它来源。
    fn load_api_key(env_source: &dyn EnvSource) -> ModelGatewayConfig {
        match env_source.read(MODEL_API_KEY_ENV_NAME) {
            None => ModelGatewayConfig {
                availability: ModelGatewayAvailability::Unavailable(
                    ModelGatewayIssue::MissingApiKey,
                ),
                required_env_name: MODEL_API_KEY_ENV_NAME,
                api_key: None,
            },
            Some(raw_value) => {
                let trimmed_value = raw_value.trim().to_string();

                if trimmed_value.is_empty() {
                    ModelGatewayConfig {
                        availability: ModelGatewayAvailability::Unavailable(
                            ModelGatewayIssue::EmptyApiKey,
                        ),
                        required_env_name: MODEL_API_KEY_ENV_NAME,
                        api_key: None,
                    }
                } else {
                    ModelGatewayConfig {
                        availability: ModelGatewayAvailability::Available,
                        required_env_name: MODEL_API_KEY_ENV_NAME,
                        api_key: Some(SensitiveString::new(trimmed_value)),
                    }
                }
            }
        }
    }
}
