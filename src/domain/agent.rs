//! 子 Agent 相关实体定义。

use serde::{Deserialize, Serialize};

/// 子 Agent 运行模式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMode {
    /// 继承上下文模式。
    Inherit,
    /// 隔离上下文模式。
    Isolate,
}

/// 子 Agent 状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStatus {
    /// 已创建未执行。
    Open,
    /// 运行中。
    Running,
    /// 已完成。
    Done,
    /// 已关闭。
    Closed,
}

/// 子 Agent 持久化实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    /// 子 Agent 标识。
    pub id: String,
    /// 父会话标识。
    pub parent_session_id: String,
    /// 父 Agent 标识。
    pub parent_agent_id: Option<String>,
    /// 运行模式。
    pub mode: AgentMode,
    /// 是否继承上下文。
    pub inherit_context: bool,
    /// 层级。
    pub level: i32,
    /// 当前状态。
    pub status: AgentStatus,
    /// 会话目录。
    pub session_dir: String,
    /// 子会话标识。
    pub child_session_id: String,
    /// 允许访问路径列表。
    pub allowed_paths_json: String,
    /// 派发任务快照。
    pub task_spec_json: String,
    /// 约束快照哈希。
    pub constraint_hash: String,
    /// 结果摘要。
    pub result_summary: String,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
}
