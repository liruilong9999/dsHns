use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use async_trait::async_trait;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: ToolParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParams {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: HashMap<String, ParamProp>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamProp {
    #[serde(rename = "type")]
    pub prop_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub call_id: String,
    pub status: ToolStatus,
    pub content: String,
    pub was_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolStatus {
    Success,
    Error { reason: String },
    Denied,
    Timeout,
    HardBlocked { reason: String },
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDef;
    async fn execute(&self, call: &ToolCall) -> ToolOutcome;
}
