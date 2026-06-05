//! 运行时领域模型。
//!
//! 本模块定义子智能体、父子关系与事件日志等运行时实体。

/// 智能体记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecord {
    /// 智能体标识。
    pub agent_id: String,
    /// 所属会话标识。
    pub session_id: String,
    /// 父智能体标识。
    pub parent_agent_id: Option<String>,
    /// 智能体模式。
    pub agent_mode: String,
    /// 当前层级。
    pub depth: i64,
    /// 当前状态。
    pub status: String,
    /// 线程映射键。
    pub thread_key: String,
    /// 当前任务摘要。
    pub task_summary: Option<String>,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
}

/// 父子智能体关系记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRelationRecord {
    /// 关系标识。
    pub relation_id: String,
    /// 父智能体标识。
    pub parent_agent_id: String,
    /// 子智能体标识。
    pub child_agent_id: String,
    /// 关系模式。
    pub relation_mode: String,
    /// 下发任务摘要。
    pub handoff_summary: Option<String>,
    /// 回传结果摘要。
    pub result_summary: Option<String>,
    /// 创建时间。
    pub created_at: String,
}

/// 事件日志记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLogRecord {
    /// 事件标识。
    pub event_id: String,
    /// 轮次标识。
    pub round_id: Option<String>,
    /// 来源会话标识。
    pub source_session_id: Option<String>,
    /// 目标会话标识。
    pub session_id: String,
    /// 事件生产者智能体标识。
    pub agent_id: Option<String>,
    /// 目标智能体标识。
    pub target_agent_id: Option<String>,
    /// 事件类型。
    pub event_type: String,
    /// 事件摘要。
    pub payload_summary: String,
    /// 事件状态。
    pub status: String,
    /// 创建时间。
    pub created_at: String,
}

/// 上下文压缩记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompressionRecord {
    /// 压缩记录标识。
    pub compression_id: String,
    /// 所属会话标识。
    pub session_id: String,
    /// 发起压缩的智能体标识。
    pub agent_id: String,
    /// 压缩起始消息标识。
    pub source_start_message_id: String,
    /// 压缩结束消息标识。
    pub source_end_message_id: String,
    /// 压缩摘要。
    pub summary_text: String,
    /// 保留消息数量。
    pub kept_message_count: i64,
    /// 触发原因。
    pub trigger_reason: String,
    /// 压缩前估算 `Token`。
    pub estimated_tokens_before: i64,
    /// 压缩后估算 `Token`。
    pub estimated_tokens_after: i64,
    /// 创建时间。
    pub created_at: String,
}
