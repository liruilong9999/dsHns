//! 工作记忆领域模型定义。

use serde::{Deserialize, Serialize};

/// 工作记忆实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemoryEntry {
    /// 记录标识。
    pub id: String,
    /// 会话标识。
    pub session_id: String,
    /// 压缩边界标识。
    pub compact_boundary_id: String,
    /// 工作记忆版本。
    pub working_memory_version: i64,
    /// 压缩前估算 Token。
    pub estimated_tokens_before: i64,
    /// 压缩后估算 Token。
    pub estimated_tokens_after: i64,
    /// 摘要正文。
    pub content: String,
    /// 创建时间。
    pub created_at: String,
}
