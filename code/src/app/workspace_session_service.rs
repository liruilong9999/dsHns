//! 目录与会话服务。
//!
//! 本模块负责承接目录实体与会话实体的服务接口，统一组合仓储能力，
//! 并按照文档契约输出目录与会话相关结果。

use crate::domain::workspace_session::{MessageRecord, WorkspaceRecord};
use crate::infra::config::{AppConfig, ApprovalMode};
use crate::infra::db::SqliteDatabase;
use crate::infra::repository::{
    MessageRepository, RepositoryError, SessionRepository, WorkspaceRepository,
};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

/// 目录自动注册请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureWorkspaceRequest {
    /// 工作区根目录路径。
    pub root_path: String,
    /// 可选展示名称；为空时根据目录名自动生成。
    pub display_name: Option<String>,
}

/// 目录重命名请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameWorkspaceRequest {
    /// 目录标识。
    pub workspace_id: String,
    /// 新的目录名称。
    pub name: String,
}

/// 会话创建请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionRequest {
    /// 归属目录标识。
    pub workspace_id: String,
    /// 首句提示词。
    pub first_prompt: String,
}

/// 会话重命名请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameSessionRequest {
    /// 会话标识。
    pub session_id: String,
    /// 新的会话名称。
    pub title: String,
}

/// 目录响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResponse {
    /// 目录标识。
    pub workspace_id: String,
    /// 展示名称。
    pub name: String,
    /// 根目录路径。
    pub root_path: String,
}

/// 目录删除响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWorkspaceResponse {
    /// 目录标识。
    pub workspace_id: String,
    /// 是否已逻辑删除。
    pub deleted: bool,
}

/// 会话创建响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCreatedResponse {
    /// 会话标识。
    pub session_id: String,
    /// 会话标题。
    pub title: String,
    /// 本次首条消息所属轮次标识。
    pub round_id: String,
    /// 首条消息是否已入队持久化。
    pub message_enqueued: bool,
}

/// 会话重命名响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRenamedResponse {
    /// 会话标识。
    pub session_id: String,
    /// 新的会话标题。
    pub title: String,
}

/// 会话列表项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListItem {
    /// 会话标识。
    pub session_id: String,
    /// 会话标题。
    pub title: String,
    /// 会话状态。
    pub status: String,
    /// 当前模型名。
    pub current_model: String,
    /// 最近消息时间。
    pub last_message_at: Option<String>,
}

/// 目录与会话服务错误。
#[derive(Debug)]
pub enum WorkspaceSessionServiceError {
    /// 输入参数不合法。
    ValidationFailed(String),
    /// 请求对象不存在。
    NotFound(String),
    /// 仓储访问失败。
    RepositoryFailed(String),
}

impl Display for WorkspaceSessionServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceSessionServiceError::ValidationFailed(message) => write!(f, "{message}"),
            WorkspaceSessionServiceError::NotFound(message) => write!(f, "{message}"),
            WorkspaceSessionServiceError::RepositoryFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for WorkspaceSessionServiceError {}

impl From<RepositoryError> for WorkspaceSessionServiceError {
    fn from(value: RepositoryError) -> Self {
        WorkspaceSessionServiceError::RepositoryFailed(value.to_string())
    }
}

/// 目录与会话服务。
pub struct WorkspaceSessionService<'a> {
    /// 数据库访问入口。
    database: &'a SqliteDatabase,
    /// 应用配置。
    config: &'a AppConfig,
}

impl<'a> WorkspaceSessionService<'a> {
    /// 构造目录与会话服务。
    pub fn new(database: &'a SqliteDatabase, config: &'a AppConfig) -> Self {
        Self { database, config }
    }

    /// 确保当前目录已经注册为目录实体。
    ///
    /// 若目录已存在则直接复用；若曾被逻辑删除，则恢复为可用目录。
    pub fn ensure_workspace(
        &self,
        request: EnsureWorkspaceRequest,
    ) -> Result<WorkspaceResponse, WorkspaceSessionServiceError> {
        let normalized_root_path = self.normalize_workspace_path(&request.root_path)?;
        let workspace_name =
            self.resolve_workspace_name(&normalized_root_path, request.display_name)?;
        let repository = WorkspaceRepository::new(self.database.connection());

        if let Some(existing_workspace) =
            repository.get_by_root_path_including_deleted(&normalized_root_path)?
        {
            let reused_workspace = if existing_workspace.is_deleted {
                repository.restore(&existing_workspace.workspace_id, &workspace_name)?
            } else {
                existing_workspace
            };

            return Ok(Self::map_workspace_response(reused_workspace));
        }

        let created_workspace = repository.create(&workspace_name, &normalized_root_path)?;
        Ok(Self::map_workspace_response(created_workspace))
    }

    /// 重命名目录实体。
    pub fn rename_workspace(
        &self,
        request: RenameWorkspaceRequest,
    ) -> Result<WorkspaceResponse, WorkspaceSessionServiceError> {
        let workspace_name = Self::require_non_empty_text(&request.name, "目录名称不能为空。")?;
        let repository = WorkspaceRepository::new(self.database.connection());
        let updated_workspace = repository.update_name(&request.workspace_id, workspace_name)?;
        Ok(Self::map_workspace_response(updated_workspace))
    }

    /// 逻辑删除目录实体，但保留历史会话数据。
    pub fn delete_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<DeleteWorkspaceResponse, WorkspaceSessionServiceError> {
        let workspace_id = Self::require_non_empty_text(workspace_id, "目录标识不能为空。")?;
        let repository = WorkspaceRepository::new(self.database.connection());
        repository.mark_deleted(workspace_id)?;

        Ok(DeleteWorkspaceResponse {
            workspace_id: workspace_id.to_string(),
            deleted: true,
        })
    }

    /// 使用首句提示词创建会话，并同步写入首条用户消息。
    pub fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<SessionCreatedResponse, WorkspaceSessionServiceError> {
        let workspace_id =
            Self::require_non_empty_text(&request.workspace_id, "目录标识不能为空。")?;
        let first_prompt =
            Self::require_non_empty_text(&request.first_prompt, "首句提示词不能为空。")?;

        let workspace_repository = WorkspaceRepository::new(self.database.connection());
        let workspace = workspace_repository
            .get_by_id_including_deleted(workspace_id)?
            .ok_or_else(|| {
                WorkspaceSessionServiceError::NotFound(format!("目录不存在：{workspace_id}"))
            })?;
        if workspace.is_deleted {
            return Err(WorkspaceSessionServiceError::ValidationFailed(format!(
                "目录已被删除，不能创建会话：{workspace_id}"
            )));
        }

        let session_repository = SessionRepository::new(self.database.connection());
        let round_id = session_repository.next_round_id()?;
        let now = SessionRepository::current_timestamp();
        let created_session = session_repository.create(
            workspace_id,
            first_prompt,
            self.config.default_model_name(),
            Self::approval_mode_to_storage(self.config.default_approval_mode()),
            i64::from(
                self.config
                    .context_limit_for_model(self.config.default_model_name())
                    .unwrap_or(256_000),
            ),
            Some(now.clone()),
            &now,
        )?;

        let message_repository = MessageRepository::new(self.database.connection());
        let _created_message: MessageRecord = message_repository.create_user_message(
            &created_session.session_id,
            &round_id,
            first_prompt,
        )?;

        Ok(SessionCreatedResponse {
            session_id: created_session.session_id,
            title: created_session.title,
            round_id,
            message_enqueued: true,
        })
    }

    /// 重命名会话。
    pub fn rename_session(
        &self,
        request: RenameSessionRequest,
    ) -> Result<SessionRenamedResponse, WorkspaceSessionServiceError> {
        let title = Self::require_non_empty_text(&request.title, "会话名称不能为空。")?;
        let repository = SessionRepository::new(self.database.connection());
        let updated_session = repository.update_title(&request.session_id, title)?;

        Ok(SessionRenamedResponse {
            session_id: updated_session.session_id,
            title: updated_session.title,
        })
    }

    /// 查询指定目录下的会话列表。
    pub fn list_sessions_by_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<SessionListItem>, WorkspaceSessionServiceError> {
        let workspace_id = Self::require_non_empty_text(workspace_id, "目录标识不能为空。")?;
        let repository = SessionRepository::new(self.database.connection());
        let sessions = repository.list_by_workspace_id(workspace_id)?;

        Ok(sessions
            .into_iter()
            .map(|session| SessionListItem {
                session_id: session.session_id,
                title: session.title,
                status: session.status,
                current_model: session.current_model,
                last_message_at: session.last_message_at,
            })
            .collect())
    }

    /// 规范化工作区路径。
    fn normalize_workspace_path(
        &self,
        raw_path: &str,
    ) -> Result<String, WorkspaceSessionServiceError> {
        let trimmed_path = Self::require_non_empty_text(raw_path, "工作区路径不能为空。")?;
        let path = PathBuf::from(trimmed_path);
        let absolute_path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| {
                    WorkspaceSessionServiceError::ValidationFailed(format!(
                        "读取当前目录失败，无法解析工作区路径：{error}"
                    ))
                })?
                .join(path)
        };

        Ok(std::fs::canonicalize(&absolute_path)
            .unwrap_or(absolute_path)
            .to_string_lossy()
            .to_string())
    }

    /// 根据路径与可选展示名确定目录名称。
    fn resolve_workspace_name(
        &self,
        normalized_root_path: &str,
        display_name: Option<String>,
    ) -> Result<String, WorkspaceSessionServiceError> {
        if let Some(name) = display_name {
            return Ok(Self::require_non_empty_text(&name, "目录名称不能为空。")?.to_string());
        }

        let path = Path::new(normalized_root_path);
        let file_name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| normalized_root_path.to_string());

        Ok(file_name)
    }

    /// 把审批模式转换为数据库持久化枚举值。
    fn approval_mode_to_storage(mode: ApprovalMode) -> &'static str {
        match mode {
            ApprovalMode::Ask => "ask",
            ApprovalMode::Auto => "auto",
            ApprovalMode::AllowAll => "allow_all",
        }
    }

    /// 校验文本非空，并返回去除首尾空白后的结果。
    fn require_non_empty_text<'b>(
        raw_value: &'b str,
        error_message: &str,
    ) -> Result<&'b str, WorkspaceSessionServiceError> {
        let trimmed_value = raw_value.trim();
        if trimmed_value.is_empty() {
            return Err(WorkspaceSessionServiceError::ValidationFailed(
                error_message.to_string(),
            ));
        }

        Ok(trimmed_value)
    }

    /// 把仓储目录记录映射为服务响应。
    fn map_workspace_response(record: WorkspaceRecord) -> WorkspaceResponse {
        WorkspaceResponse {
            workspace_id: record.workspace_id,
            name: record.name,
            root_path: record.root_path,
        }
    }
}
