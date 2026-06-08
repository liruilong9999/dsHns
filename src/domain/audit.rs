//! 审计相关实体定义。

use serde::{Deserialize, Serialize};

use crate::utils::time::now_rfc3339;

/// 删除与恢复审计实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletionAudit {
    /// 审计记录唯一标识。
    pub id: String,
    /// 目标类型，例如 `session` 或 `workspace_directory`。
    pub target_type: String,
    /// 目标标识。
    pub target_id: String,
    /// 删除模式。
    pub delete_mode: String,
    /// 产物是否缺失。
    pub artifact_missing: bool,
    /// 操作来源。
    pub operator: String,
    /// 用于恢复的快照载荷。
    pub payload_json: String,
    /// 创建时间。
    pub created_at: String,
    /// 恢复时间。
    pub restored_at: Option<String>,
}

impl DeletionAudit {
    /// 创建新的审计记录。
    pub fn new(
        target_type: impl Into<String>,
        target_id: impl Into<String>,
        delete_mode: impl Into<String>,
        artifact_missing: bool,
        operator: impl Into<String>,
        payload_json: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            target_type: target_type.into(),
            target_id: target_id.into(),
            delete_mode: delete_mode.into(),
            artifact_missing,
            operator: operator.into(),
            payload_json: payload_json.into(),
            created_at: now_rfc3339(),
            restored_at: None,
        }
    }
}
