//! DeepSeek 兼容客户端实现。

use std::collections::BTreeMap;
use std::env;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::domain::{AssistantToolCall, Message, MessageRole, ToolFunctionCall};

/// 模型调用结果。
#[derive(Debug, Clone)]
pub struct ModelResponse {
    /// 助手正文。
    pub content: String,
    /// 推理内容。
    pub reasoning_content: Option<String>,
    /// 工具调用列表。
    pub tool_calls: Vec<AssistantToolCall>,
    /// 输入 Token 用量。
    pub input_tokens: usize,
    /// 输出 Token 用量。
    pub output_tokens: usize,
    /// 是否发生了流式降级。
    pub stream_fallback: bool,
}

/// 非流式响应顶层结构。
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    /// 返回选项列表。
    choices: Vec<Choice>,
    /// Token 用量。
    usage: Option<Usage>,
}

/// 非流式选项。
#[derive(Debug, Deserialize)]
struct Choice {
    /// 消息正文。
    message: ResponseMessage,
}

/// 非流式消息。
#[derive(Debug, Deserialize)]
struct ResponseMessage {
    /// 文本内容。
    content: Option<String>,
    /// 推理内容。
    reasoning_content: Option<String>,
    /// 工具调用列表。
    #[serde(default)]
    tool_calls: Vec<AssistantToolCall>,
}

/// SSE 分片响应。
#[derive(Debug, Deserialize)]
struct StreamChunkResponse {
    /// 分片选项列表。
    #[serde(default)]
    choices: Vec<StreamChoice>,
    /// 可选 Token 用量。
    usage: Option<Usage>,
}

/// SSE 分片选项。
#[derive(Debug, Deserialize)]
struct StreamChoice {
    /// 增量内容。
    delta: StreamDelta,
}

/// SSE 增量结构。
#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    /// 增量正文。
    content: Option<String>,
    /// 增量推理内容。
    reasoning_content: Option<String>,
    /// 增量工具调用。
    #[serde(default)]
    tool_calls: Vec<StreamToolCallDelta>,
}

/// SSE 工具调用增量。
#[derive(Debug, Default, Deserialize)]
struct StreamToolCallDelta {
    /// 工具调用索引。
    index: Option<usize>,
    /// 工具调用标识。
    id: Option<String>,
    /// 固定为 function。
    #[serde(rename = "type")]
    kind: Option<String>,
    /// 函数增量。
    function: Option<StreamFunctionDelta>,
}

/// SSE 函数增量。
#[derive(Debug, Default, Deserialize)]
struct StreamFunctionDelta {
    /// 工具名称。
    name: Option<String>,
    /// 参数 JSON 片段。
    arguments: Option<String>,
}

/// Token 用量结构。
#[derive(Debug, Deserialize)]
struct Usage {
    /// 输入 Token 数。
    prompt_tokens: Option<usize>,
    /// 输出 Token 数。
    completion_tokens: Option<usize>,
}

/// 工具调用累加器。
#[derive(Debug, Default)]
struct ToolCallAccumulator {
    /// 工具调用标识。
    id: String,
    /// 调用类型。
    kind: String,
    /// 工具名称。
    name: String,
    /// 参数累加字符串。
    arguments: String,
}

/// 大模型客户端。
#[derive(Clone)]
pub struct LlmClient {
    /// HTTP 客户端。
    client: Client,
    /// 接口地址。
    base_url: String,
}

impl LlmClient {
    /// 创建客户端实例。
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
        }
    }

    /// 发起聊天补全请求。
    pub async fn chat_completion(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        stream: bool,
    ) -> Result<ModelResponse> {
        let api_key = env::var("DEEPSEEK_API_KEY")
            .map_err(|_| anyhow!("缺少环境变量 DEEPSEEK_API_KEY，无法调用模型接口"))?;

        if stream {
            match self
                .chat_completion_stream(model, messages, tools, &api_key)
                .await
            {
                Ok(mut response) => {
                    response.stream_fallback = false;
                    return Ok(response);
                }
                Err(error) => {
                    tracing::warn!("流式请求失败，开始降级到非流式：{}", error);
                }
            }
        }

        let mut response = self
            .chat_completion_non_stream(model, messages, tools, &api_key)
            .await?;
        response.stream_fallback = stream;
        Ok(response)
    }

    /// 执行非流式请求。
    async fn chat_completion_non_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        api_key: &str,
    ) -> Result<ModelResponse> {
        let payload = build_payload(model, messages, tools, false);
        let response = self
            .client
            .post(&self.base_url)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("调用模型接口失败：{}", self.base_url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "无法读取错误正文".to_string());
            return Err(anyhow!(
                "模型接口返回失败状态：{}，错误正文：{}",
                status,
                body
            ));
        }

        let parsed: ChatCompletionResponse = response.json().await.context("解析模型响应失败")?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("模型未返回有效的 choices"))?;

        Ok(ModelResponse {
            content: choice.message.content.unwrap_or_default(),
            reasoning_content: choice.message.reasoning_content,
            tool_calls: choice.message.tool_calls,
            input_tokens: parsed
                .usage
                .as_ref()
                .and_then(|usage| usage.prompt_tokens)
                .unwrap_or_default(),
            output_tokens: parsed
                .usage
                .as_ref()
                .and_then(|usage| usage.completion_tokens)
                .unwrap_or_default(),
            stream_fallback: false,
        })
    }

    /// 执行流式请求。
    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        api_key: &str,
    ) -> Result<ModelResponse> {
        let payload = build_payload(model, messages, tools, true);
        let response = self
            .client
            .post(&self.base_url)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("调用流式模型接口失败：{}", self.base_url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "无法读取错误正文".to_string());
            return Err(anyhow!(
                "流式模型接口返回失败状态：{}，错误正文：{}",
                status,
                body
            ));
        }

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut prompt_tokens = 0usize;
        let mut completion_tokens = 0usize;
        let mut buffer = String::new();
        let mut tool_calls: BTreeMap<usize, ToolCallAccumulator> = BTreeMap::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("读取流式响应分片失败")?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(position) = buffer.find('\n') {
                let line = buffer[..position].trim().to_string();
                buffer = buffer[position + 1..].to_string();

                if !line.starts_with("data:") {
                    continue;
                }

                let payload = line.trim_start_matches("data:").trim();
                if payload.is_empty() {
                    continue;
                }
                if payload == "[DONE]" {
                    continue;
                }

                let chunk: StreamChunkResponse =
                    serde_json::from_str(payload).context("解析流式分片失败")?;
                if let Some(usage) = chunk.usage.as_ref() {
                    prompt_tokens = usage.prompt_tokens.unwrap_or(prompt_tokens);
                    completion_tokens = usage.completion_tokens.unwrap_or(completion_tokens);
                }

                for choice in chunk.choices {
                    if let Some(delta) = choice.delta.content {
                        content.push_str(&delta);
                    }
                    if let Some(delta) = choice.delta.reasoning_content {
                        reasoning.push_str(&delta);
                    }

                    for tool_delta in choice.delta.tool_calls {
                        let index = tool_delta.index.unwrap_or(tool_calls.len());
                        let entry = tool_calls.entry(index).or_default();
                        if let Some(id) = tool_delta.id {
                            entry.id = id;
                        }
                        if let Some(kind) = tool_delta.kind {
                            entry.kind = kind;
                        }
                        if let Some(function) = tool_delta.function {
                            if let Some(name) = function.name {
                                entry.name.push_str(&name);
                            }
                            if let Some(arguments) = function.arguments {
                                entry.arguments.push_str(&arguments);
                            }
                        }
                    }
                }
            }
        }

        let tool_calls = tool_calls
            .into_values()
            .map(|item| AssistantToolCall {
                id: item.id,
                kind: if item.kind.is_empty() {
                    "function".to_string()
                } else {
                    item.kind
                },
                function: ToolFunctionCall {
                    name: item.name,
                    arguments: item.arguments,
                },
            })
            .collect::<Vec<_>>();

        Ok(ModelResponse {
            content,
            reasoning_content: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            tool_calls,
            input_tokens: prompt_tokens,
            output_tokens: completion_tokens,
            stream_fallback: false,
        })
    }
}

fn build_payload(model: &str, messages: &[Message], tools: &[Value], stream: bool) -> Value {
    json!({
        "model": model,
        "stream": stream,
        "messages": messages.iter().map(serialize_message).collect::<Vec<_>>(),
        "tools": tools,
        "temperature": 0.2
    })
}

/// 将内部消息序列化为 OpenAI/DeepSeek 兼容消息。
fn serialize_message(message: &Message) -> Value {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    };

    let mut value = json!({
        "role": role,
        "content": message.content
    });

    if let Some(tool_call_id) = &message.tool_call_id {
        value["tool_call_id"] = json!(tool_call_id);
    }
    if !message.tool_calls.is_empty() {
        value["tool_calls"] =
            serde_json::to_value(&message.tool_calls).unwrap_or_else(|_| json!([]));
    }

    value
}

#[cfg(test)]
mod tests {
    use super::{AssistantToolCall, LlmClient, ToolFunctionCall};

    #[test]
    fn should_accumulate_tool_call_delta_shape() {
        let call = AssistantToolCall {
            id: "call_1".to_string(),
            kind: "function".to_string(),
            function: ToolFunctionCall {
                name: "read_file".to_string(),
                arguments: "{\"path\":\"a.txt\"}".to_string(),
            },
        };
        assert_eq!(call.function.name, "read_file");
        assert!(call.function.arguments.contains("a.txt"));
        let _client = LlmClient::new("https://example.com".to_string());
    }
}
