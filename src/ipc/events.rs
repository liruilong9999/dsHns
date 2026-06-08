//! IPC 事件契约定义。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 事件类型枚举。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    /// 会话状态变化事件。
    SessionStatusChanged,
    /// 流式文本块事件。
    StreamChunkReceived,
    /// 审批请求事件。
    ApprovalRequested,
    /// 审批完成事件。
    ApprovalResolved,
    /// 工具状态变化事件。
    ToolStatusChanged,
    /// Token 统计更新事件。
    TokenUsageUpdated,
    /// 工作记忆生成事件。
    WorkingMemoryCreated,
    /// 子 Agent 状态变化事件。
    AgentStatusChanged,
}

/// IPC 事件载体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcEvent {
    /// 事件唯一标识。
    pub event_id: String,
    /// 会话标识。
    pub session_id: String,
    /// 事件时间。
    pub timestamp: String,
    /// 所属轮次。
    pub round_no: i64,
    /// 事件类型。
    pub event_type: EventType,
    /// 事件载荷。
    pub payload: Value,
}
