use crate::tool::{ToolCall, ToolStatus};
use crate::message::Usage;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    UserInput(String),
    Thinking(String),
    ToolCallStart { id: String, name: String },
    ToolBlocked { call: ToolCall, reason: String },
    ToolExecution { call_id: String, status: ToolStatus, summary: String },
    ToolConfirmationNeeded { call: ToolCall, reason: String },
    SubAgentOpened { agent_id: String, mode: String, description: String },
    SubAgentCompleted { agent_id: String, summary: String },
    SubAgentClosed { agent_id: String },
    TurnComplete { usage: Usage, tool_rounds: u32 },
    SessionComplete,
    Error(String),
}
