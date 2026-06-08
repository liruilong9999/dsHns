//! 会话实体定义。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::utils::time::now_rfc3339;

/// 审批模式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalMode {
    /// 所有工具都需要用户确认。
    AskUser,
    /// 只自动放行低风险只读工具。
    AutoApproveSafe,
    /// 放行全部工具。
    FullAccess,
}

impl ApprovalMode {
    /// 返回用于持久化的稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskUser => "AskUser",
            Self::AutoApproveSafe => "AutoApproveSafe",
            Self::FullAccess => "FullAccess",
        }
    }

    /// 从持久化字符串恢复审批模式。
    pub fn from_str(value: &str) -> Self {
        match value {
            "AutoApproveSafe" => Self::AutoApproveSafe,
            "FullAccess" => Self::FullAccess,
            _ => Self::AskUser,
        }
    }
}

/// 会话运行状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionStatus {
    /// 未选中状态。
    Unselected,
    /// 已选中待执行状态。
    Selected,
    /// 正在执行状态。
    Running,
    /// 已取消状态。
    Cancelled,
    /// 已结束状态。
    Finished,
}

impl SessionStatus {
    /// 返回用于持久化的稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unselected => "unselected",
            Self::Selected => "selected",
            Self::Running => "running",
            Self::Cancelled => "cancelled",
            Self::Finished => "finished",
        }
    }

    /// 从持久化字符串恢复状态。
    pub fn from_str(value: &str) -> Self {
        match value {
            "selected" => Self::Selected,
            "running" => Self::Running,
            "cancelled" => Self::Cancelled,
            "finished" => Self::Finished,
            _ => Self::Unselected,
        }
    }
}

/// 会话实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// 会话唯一标识。
    pub id: String,
    /// 所属目录标识。
    pub directory_id: String,
    /// 会话名称。
    pub name: String,
    /// 项目名称快照。
    pub project_name: String,
    /// 项目路径快照。
    pub project_path: String,
    /// 当前工作目录。
    pub working_directory: String,
    /// 当前使用的模型名称。
    pub model: String,
    /// 当前审批模式。
    pub approval_mode: ApprovalMode,
    /// 当前状态。
    pub status: SessionStatus,
    /// 是否启用流式输出。
    pub stream_output: bool,
    /// 当前轮次。
    pub round: i64,
    /// 是否已结束。
    pub is_finished: bool,
    /// 会话目录路径。
    pub session_dir: PathBuf,
    /// 最近快照版本号。
    pub snapshot_version: i64,
    /// 最近持久化轮次。
    pub last_round_no: i64,
    /// 最近快照哈希。
    pub content_hash: String,
    /// 当前系统提示词快照。
    pub system_prompt: String,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
}

impl Session {
    /// 创建新的会话实体。
    pub fn new(
        id: String,
        directory_id: String,
        name: String,
        project_name: String,
        project_path: String,
        working_directory: String,
        model: String,
        approval_mode: ApprovalMode,
        stream_output: bool,
        session_dir: PathBuf,
        system_prompt: String,
    ) -> Self {
        let now = now_rfc3339();
        Self {
            id,
            directory_id,
            name,
            project_name,
            project_path,
            working_directory,
            model,
            approval_mode,
            status: SessionStatus::Selected,
            stream_output,
            round: 0,
            is_finished: false,
            session_dir,
            snapshot_version: 1,
            last_round_no: 0,
            content_hash: String::new(),
            system_prompt,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}
