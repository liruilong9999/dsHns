//! 领域模型模块。

/// Agent 相关领域模型。
pub mod agent;
/// 审计相关领域模型。
pub mod audit;
/// 消息相关领域模型。
pub mod message;
/// 会话相关领域模型。
pub mod session;
/// 状态快照领域模型。
pub mod status;
/// 工具相关领域模型。
pub mod tool;
/// 工作记忆领域模型。
pub mod working_memory;
/// 工作区相关领域模型。
pub mod workspace;

pub use agent::{AgentInstance, AgentMode, AgentStatus};
pub use audit::DeletionAudit;
pub use message::{AssistantToolCall, Message, MessageRole, ToolFunctionCall};
pub use session::{ApprovalMode, Session, SessionStatus};
pub use status::SessionStatusSnapshot;
pub use tool::{
    ReplaceRange, ToolCallRecord, ToolFailureType, ToolProjectionType, ToolResultRecord,
    ToolRiskLevel,
};
pub use working_memory::WorkingMemoryEntry;
pub use workspace::WorkspaceDirectory;
