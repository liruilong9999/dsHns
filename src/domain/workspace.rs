//! 工作区实体定义。

use serde::{Deserialize, Serialize};

/// 工作区目录实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDirectory {
    /// 目录唯一标识。
    pub id: String,
    /// 项目名称。
    pub project_name: String,
    /// 项目绝对路径。
    pub project_path: String,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
    /// 是否已软删除。
    pub is_deleted: bool,
}
