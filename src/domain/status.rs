//! 状态快照领域模型定义。

use serde::{Deserialize, Serialize};

use crate::domain::Session;

/// 会话状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusSnapshot {
    /// 会话信息。
    pub session: Session,
    /// 最近一次输入 Token。
    pub input_tokens: usize,
    /// 最近一次输出 Token。
    pub output_tokens: usize,
    /// 缓存命中率。
    pub cache_hit_rate: f64,
    /// 剩余上下文预算。
    pub remaining_context: usize,
}
