//! 目录、会话与消息领域模型。
//!
//! 当前阶段先覆盖 `TASK-005` 到 `TASK-007` 所需的基础实体。

/// 目录记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    /// 目录标识。
    pub workspace_id: String,
    /// 展示名称。
    pub name: String,
    /// 工作区根目录绝对路径。
    pub root_path: String,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
    /// 是否已逻辑删除。
    pub is_deleted: bool,
}

/// 会话记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// 会话标识。
    pub session_id: String,
    /// 归属目录标识。
    pub workspace_id: String,
    /// 会话标题。
    pub title: String,
    /// 会话状态。
    pub status: String,
    /// 当前模型名。
    pub current_model: String,
    /// 会话审批模式。
    pub session_approval_mode: String,
    /// 上下文上限。
    pub context_limit: i64,
    /// 最后消息时间。
    pub last_message_at: Option<String>,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
}

/// 消息记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecord {
    /// 消息标识。
    pub message_id: String,
    /// 归属会话标识。
    pub session_id: String,
    /// 产生该消息的智能体标识。
    pub agent_id: Option<String>,
    /// 所属轮次标识。
    pub round_id: String,
    /// 会话内顺序号。
    pub sequence_no: i64,
    /// 消息角色。
    pub role: String,
    /// 消息内容。
    pub content: String,
    /// 内容类型。
    pub content_type: String,
    /// 估算 Token 数。
    pub token_estimate: i64,
    /// 是否进入模型上下文。
    pub include_in_context: bool,
    /// 是否已经成为压缩源消息。
    pub is_compressed_source: bool,
    /// 创建时间。
    pub created_at: String,
}
