use crate::tool::{ToolCall, ToolStatus};
use crate::message::Usage;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum AgentEvent {
    UserInput(String),
    Thinking(String),
    ToolCallStart { id: String, name: String },
    ToolBlocked { call: ToolCall, reason: String },
    ToolExecution { call_id: String, status: ToolStatus, summary: String },
    /// 需要用户确认，通过 response_tx 回复 true=批准 / false=拒绝
    ToolConfirmationNeeded {
        call: ToolCall,
        reason: String,
        response_tx: oneshot::Sender<bool>,
    },
    SubAgentOpened { agent_id: String, mode: String, description: String },
    SubAgentCompleted { agent_id: String, summary: String },
    SubAgentClosed { agent_id: String },
    TurnComplete { usage: Usage, tool_rounds: u32 },
    SessionComplete,
    Error(String),
}
