//! 工具领域模型。
//!
//! 本模块定义工具元数据、参数模式、请求响应结构与审批枚举，
//! 供工具注册中心与工具调度器统一复用。

use serde_json::Value;
use std::collections::BTreeMap;

/// 会话审批模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionApprovalMode {
    /// 需要人工确认。
    Ask,
    /// 自动执行允许的工具。
    Auto,
    /// 跳过人工确认，但不跳过参数与边界校验。
    AllowAll,
}

/// 工具默认权限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPermission {
    /// 默认允许。
    Allow,
    /// 默认拒绝，只有 `allow_all` 可直接跳过人工确认。
    Deny,
    /// 仅允许在工作区范围内访问。
    WorkspaceOnly,
}

/// 工具执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionStatus {
    /// 工具执行成功。
    Success,
    /// 工具执行失败。
    Failed,
    /// 工具被阻止。
    Blocked,
}

/// 参数值类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolValueType {
    /// 字符串。
    String,
    /// 整数。
    Integer,
    /// 布尔值。
    Boolean,
    /// 对象。
    Object,
    /// 数组。
    Array,
}

/// 参数模式节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchemaNode {
    /// 当前节点值类型。
    pub value_type: ToolValueType,
    /// 对象类型的必填字段。
    pub required: Vec<String>,
    /// 对象类型的属性定义。
    pub properties: BTreeMap<String, ToolSchemaNode>,
    /// 数组类型的元素定义。
    pub items: Option<Box<ToolSchemaNode>>,
}

impl ToolSchemaNode {
    /// 创建对象节点。
    pub fn object(required: Vec<&str>, properties: BTreeMap<String, ToolSchemaNode>) -> Self {
        Self {
            value_type: ToolValueType::Object,
            required: required.into_iter().map(ToString::to_string).collect(),
            properties,
            items: None,
        }
    }

    /// 创建字符串节点。
    pub fn string() -> Self {
        Self {
            value_type: ToolValueType::String,
            required: Vec::new(),
            properties: BTreeMap::new(),
            items: None,
        }
    }

    /// 创建整数节点。
    pub fn integer() -> Self {
        Self {
            value_type: ToolValueType::Integer,
            required: Vec::new(),
            properties: BTreeMap::new(),
            items: None,
        }
    }

    /// 创建布尔值节点。
    pub fn boolean() -> Self {
        Self {
            value_type: ToolValueType::Boolean,
            required: Vec::new(),
            properties: BTreeMap::new(),
            items: None,
        }
    }

    /// 创建数组节点。
    pub fn array(items: ToolSchemaNode) -> Self {
        Self {
            value_type: ToolValueType::Array,
            required: Vec::new(),
            properties: BTreeMap::new(),
            items: Some(Box::new(items)),
        }
    }
}

/// 工具元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolMetadata {
    /// 工具名称。
    pub name: String,
    /// 工具描述。
    pub description: String,
    /// 参数模式定义。
    pub schema: ToolSchemaNode,
    /// 默认权限。
    pub default_permission: ToolPermission,
    /// 是否向模型与用户显示。
    pub visible: bool,
    /// 是否允许后台运行。
    pub background: bool,
    /// 执行器映射键。
    pub executor_key: String,
}

/// 工具调用请求。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRequest {
    /// 工具名。
    pub tool_name: String,
    /// 会话标识。
    pub session_id: String,
    /// 智能体标识。
    pub agent_id: String,
    /// 轮次标识。
    pub round_id: String,
    /// 调用参数。
    pub arguments: Value,
}

impl ToolCallRequest {
    /// 构造工具调用请求。
    pub fn new(
        tool_name: &str,
        session_id: &str,
        agent_id: &str,
        round_id: &str,
        arguments: Value,
    ) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            round_id: round_id.to_string(),
            arguments,
        }
    }
}

/// 工具响应。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResponse {
    /// 是否执行成功。
    pub ok: bool,
    /// 执行状态。
    pub status: ToolExecutionStatus,
    /// 工具名。
    pub tool_name: String,
    /// 会话标识。
    pub session_id: String,
    /// 智能体标识。
    pub agent_id: String,
    /// 是否显式展示。
    pub visible: bool,
    /// 退出码。
    pub exit_code: Option<i32>,
    /// 结果摘要。
    pub result_summary: Option<String>,
    /// 结果载荷。
    pub result_payload: Value,
    /// 错误类型。
    pub error_type: Option<String>,
    /// 错误码。
    pub error_code: Option<String>,
    /// 中文错误说明。
    pub message: Option<String>,
    /// 是否可重试。
    pub retryable: bool,
}

impl ToolResponse {
    /// 构造成功响应。
    pub fn success(
        request: &ToolCallRequest,
        visible: bool,
        exit_code: Option<i32>,
        result_summary: impl Into<String>,
        result_payload: Value,
    ) -> Self {
        Self {
            ok: true,
            status: ToolExecutionStatus::Success,
            tool_name: request.tool_name.clone(),
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
            visible,
            exit_code,
            result_summary: Some(result_summary.into()),
            result_payload,
            error_type: None,
            error_code: None,
            message: None,
            retryable: false,
        }
    }

    /// 构造失败响应。
    pub fn failed(
        request: &ToolCallRequest,
        visible: bool,
        error_type: &str,
        error_code: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            ok: false,
            status: ToolExecutionStatus::Failed,
            tool_name: request.tool_name.clone(),
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
            visible,
            exit_code: None,
            result_summary: None,
            result_payload: Value::Null,
            error_type: Some(error_type.to_string()),
            error_code: Some(error_code.to_string()),
            message: Some(message.into()),
            retryable,
        }
    }

    /// 构造阻止响应。
    pub fn blocked(
        request: &ToolCallRequest,
        visible: bool,
        error_type: &str,
        error_code: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            ok: false,
            status: ToolExecutionStatus::Blocked,
            tool_name: request.tool_name.clone(),
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
            visible,
            exit_code: None,
            result_summary: None,
            result_payload: Value::Null,
            error_type: Some(error_type.to_string()),
            error_code: Some(error_code.to_string()),
            message: Some(message.into()),
            retryable,
        }
    }
}
