//! DeepSeek 实时模型网关。
//!
//! 本模块把本地 `AgentRunner` 的消息与工具定义转换为 DeepSeek
//! `chat/completions` 请求，并解析工具调用与最终输出。

use crate::app::agent_runner::{
    ModelGatewayError, ModelGatewayRequest, ModelGatewayResponse, ModelGatewayTrait,
};
use crate::infra::config::SensitiveString;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// DeepSeek 模型网关。
#[derive(Clone)]
pub struct DeepSeekGateway {
    client: Client,
    api_key: SensitiveString,
    base_url: String,
}

impl DeepSeekGateway {
    /// 构造默认 DeepSeek 网关。
    pub fn new(api_key: SensitiveString) -> Result<Self, ModelGatewayError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| {
                ModelGatewayError::RequestFailed(format!("创建 DeepSeek HTTP 客户端失败：{error}"))
            })?;
        Ok(Self {
            client,
            api_key,
            base_url: "https://api.deepseek.com".to_string(),
        })
    }
}

impl ModelGatewayTrait for DeepSeekGateway {
    fn complete(
        &self,
        request: ModelGatewayRequest,
    ) -> Result<ModelGatewayResponse, ModelGatewayError> {
        let has_tools = !request.tools.is_empty();
        let body = DeepSeekChatRequest {
            model: request.model_name,
            messages: request.messages,
            tools: if has_tools { Some(request.tools) } else { None },
            tool_choice: if has_tools {
                Some("auto".to_string())
            } else {
                None
            },
            max_tokens: Some(request.max_tokens),
            stream: Some(false),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(self.api_key.expose())
            .json(&body)
            .send()
            .map_err(|error| {
                ModelGatewayError::RequestFailed(format!("请求 DeepSeek 接口失败：{error}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().unwrap_or_else(|_| "读取错误响应失败".to_string());
            return Err(ModelGatewayError::RequestFailed(format!(
                "DeepSeek 接口返回失败状态：{}，响应：{}",
                status, body_text
            )));
        }

        let payload = response.json::<DeepSeekChatResponse>().map_err(|error| {
            ModelGatewayError::RequestFailed(format!("解析 DeepSeek 响应失败：{error}"))
        })?;

        let choice = payload.choices.into_iter().next().ok_or_else(|| {
            ModelGatewayError::RequestFailed("DeepSeek 响应中缺少 choices。".to_string())
        })?;

        if let Some(tool_call) = choice.message.tool_calls.and_then(|mut items| items.drain(..).next()) {
            let arguments = serde_json::from_str::<Value>(&tool_call.function.arguments).map_err(|error| {
                ModelGatewayError::RequestFailed(format!("解析工具调用参数失败：{error}"))
            })?;
            return Ok(ModelGatewayResponse::ToolCall {
                tool_name: tool_call.function.name,
                arguments,
                tool_call_id: tool_call.id,
                assistant_content: choice.message.content,
                reasoning_content: choice.message.reasoning_content,
            });
        }

        if let Some(content) = choice.message.content {
            return Ok(ModelGatewayResponse::FinalText {
                content,
                reasoning_content: choice.message.reasoning_content,
            });
        }

        Err(ModelGatewayError::RequestFailed(
            "DeepSeek 响应既没有最终文本，也没有工具调用。".to_string(),
        ))
    }
}

#[derive(Debug, Serialize)]
struct DeepSeekChatRequest {
    model: String,
    messages: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChatResponse {
    choices: Vec<DeepSeekChoice>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    message: DeepSeekMessage,
}

#[derive(Debug, Deserialize)]
struct DeepSeekMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<DeepSeekToolCall>>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekToolCall {
    id: String,
    function: DeepSeekToolFunction,
}

#[derive(Debug, Deserialize)]
struct DeepSeekToolFunction {
    name: String,
    arguments: String,
}
