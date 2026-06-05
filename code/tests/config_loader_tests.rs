//! 配置加载模块测试。
//!
//! 这些测试覆盖默认模型、上下文限制、审批模式、环境变量异常和敏感信息保护要求。

use dshns_agent::infra::config::{
    AppConfig, ApprovalMode, EnvSource, ModelGatewayAvailability, ModelGatewayIssue,
};
use std::collections::HashMap;

/// 测试使用的假环境变量读取器，避免直接污染进程环境。
struct FakeEnvSource {
    /// 预置的环境变量键值对。
    values: HashMap<String, String>,
}

impl FakeEnvSource {
    /// 使用给定键值对构造测试环境变量读取器。
    fn new(values: impl IntoIterator<Item = (&'static str, &'static str)>) -> Self {
        Self {
            values: values
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }
}

impl EnvSource for FakeEnvSource {
    fn read(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }
}

#[test]
fn 应加载文档约定的默认模型与审批模式() {
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "abc123")]));

    assert_eq!(config.default_model_name(), "deepseek-v4-flash");
    assert_eq!(config.default_approval_mode(), ApprovalMode::Ask);
    assert_eq!(config.available_model_names().len(), 4);
    assert!(
        config
            .available_model_names()
            .contains(&"deepseek-v4-pro[1m]".to_string())
    );
}

#[test]
fn 应为普通模型和一百万上下文模型设置默认限制() {
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "abc123")]));

    assert_eq!(
        config.context_limit_for_model("deepseek-v4-pro"),
        Some(256_000)
    );
    assert_eq!(
        config.context_limit_for_model("deepseek-v4-pro[1m]"),
        Some(1_000_000)
    );
}

#[test]
fn 缺失环境变量时应把模型网关标记为不可用并返回中文原因() {
    let config = AppConfig::load_from_env(&FakeEnvSource::new([]));

    assert_eq!(
        config.model_gateway().availability(),
        &ModelGatewayAvailability::Unavailable(ModelGatewayIssue::MissingApiKey)
    );
    assert_eq!(
        config.model_gateway().user_facing_message(),
        "环境变量 DEEPSEEK_API_KEY 缺失，模型网关不可用，请设置后重启应用。"
    );
}

#[test]
fn 环境变量为空白时应把模型网关标记为不可用并返回中文原因() {
    let config = AppConfig::load_from_env(&FakeEnvSource::new([("DEEPSEEK_API_KEY", "   ")]));

    assert_eq!(
        config.model_gateway().availability(),
        &ModelGatewayAvailability::Unavailable(ModelGatewayIssue::EmptyApiKey)
    );
    assert_eq!(
        config.model_gateway().user_facing_message(),
        "环境变量 DEEPSEEK_API_KEY 去除首尾空白后为空，模型网关不可用，请设置有效值后重启应用。"
    );
}

#[test]
fn 调试输出不应泄漏敏感环境变量明文() {
    let config = AppConfig::load_from_env(&FakeEnvSource::new([(
        "DEEPSEEK_API_KEY",
        "super-secret-key",
    )]));

    let debug_output = format!("{config:?}");

    assert!(!debug_output.contains("super-secret-key"));
    assert!(debug_output.contains("[已隐藏的敏感信息]"));
}
