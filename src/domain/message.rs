//! 消息实体定义。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 消息角色。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// 系统消息。
    System,
    /// 用户消息。
    User,
    /// 助手消息。
    Assistant,
    /// 工具消息。
    Tool,
}

/// 工具函数载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunctionCall {
    /// 工具名称。
    pub name: String,
    /// JSON 字符串形式的参数。
    pub arguments: String,
}

/// 助手消息中的工具调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantToolCall {
    /// 工具调用唯一标识。
    pub id: String,
    /// 工具调用类型，固定为 function。
    #[serde(rename = "type")]
    pub kind: String,
    /// 工具函数定义。
    pub function: ToolFunctionCall,
}

/// 通用消息实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// 消息角色。
    pub role: MessageRole,
    /// 消息正文。
    pub content: String,
    /// 可选名称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 工具消息关联的 tool_call_id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 助手消息携带的工具调用列表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AssistantToolCall>,
    /// 推理内容。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// 本地元数据。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl Message {
    /// 创建系统消息。
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
            reasoning_content: None,
            metadata: None,
        }
    }

    /// 创建用户消息。
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
            reasoning_content: None,
            metadata: None,
        }
    }

    /// 创建助手消息。
    pub fn assistant(
        content: impl Into<String>,
        tool_calls: Vec<AssistantToolCall>,
        reasoning_content: Option<String>,
    ) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls,
            reasoning_content,
            metadata: None,
        }
    }

    /// 创建工具消息。
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            name: None,
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: Vec::new(),
            reasoning_content: None,
            metadata: None,
        }
    }
}
