//! Tauri IPC 预留契约实现。
//!
//! 当前阶段为未来前端接入定义稳定的命令名、请求结构与响应结构；
//! 流式订阅保持未实现，目录/会话类命令可直接复用后端服务能力。

use crate::app::workspace_session_service::{
    AppendUserMessageRequest, ChangeSessionApprovalModeRequest, ChangeSessionModelRequest,
    CreateSessionRequest, EnsureWorkspaceRequest, RenameSessionRequest, RenameWorkspaceRequest,
    WorkspaceSessionService, WorkspaceSessionServiceError,
};
use crate::infra::event_bus::{EventBus, EventEnvelope, EventType};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCreateRequest {
    pub workspace_path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceUpdateRequest {
    pub workspace_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDeleteRequest {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCreateRequest {
    pub workspace_id: String,
    pub first_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListRequest {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUpdateRequest {
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSendMessageRequest {
    pub session_id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSwitchModelRequest {
    pub session_id: String,
    pub model_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSwitchApprovalModeRequest {
    pub session_id: String,
    pub target_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSubscribeStreamRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcWorkspaceResponse {
    pub ok: bool,
    pub workspace_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcWorkspaceDeleteResponse {
    pub ok: bool,
    pub workspace_id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSessionCreateResponse {
    pub ok: bool,
    pub session_id: String,
    pub title: String,
    pub round_id: String,
    pub message_enqueued: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSessionListResponse {
    pub ok: bool,
    pub workspace_id: String,
    pub sessions: Vec<crate::app::workspace_session_service::SessionListItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSessionUpdateResponse {
    pub ok: bool,
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSessionSendMessageResponse {
    pub ok: bool,
    pub session_id: String,
    pub round_id: String,
    pub message_enqueued: bool,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSessionSwitchModelResponse {
    pub ok: bool,
    pub session_id: String,
    pub current_model: String,
    pub context_limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSessionSwitchApprovalModeResponse {
    pub ok: bool,
    pub session_id: String,
    pub session_approval_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcNotImplementedError {
    pub ok: bool,
    pub error_code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug)]
pub enum TauriIpcError {
    ServiceFailed(String),
    NotImplemented(IpcNotImplementedError),
}

impl Display for TauriIpcError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TauriIpcError::ServiceFailed(message) => write!(f, "{message}"),
            TauriIpcError::NotImplemented(error) => write!(f, "{}", error.message),
        }
    }
}

impl Error for TauriIpcError {}

impl From<WorkspaceSessionServiceError> for TauriIpcError {
    fn from(value: WorkspaceSessionServiceError) -> Self {
        TauriIpcError::ServiceFailed(value.to_string())
    }
}

pub struct TauriIpcFacade<'a> {
    workspace_session_service: WorkspaceSessionService<'a>,
    event_bus: EventBus<'a>,
}

impl<'a> TauriIpcFacade<'a> {
    pub fn new(
        workspace_session_service: WorkspaceSessionService<'a>,
        event_bus: EventBus<'a>,
    ) -> Self {
        Self {
            workspace_session_service,
            event_bus,
        }
    }

    pub fn workspace_create(
        &self,
        request: WorkspaceCreateRequest,
    ) -> Result<IpcWorkspaceResponse, TauriIpcError> {
        let response = self
            .workspace_session_service
            .ensure_workspace(EnsureWorkspaceRequest {
                root_path: request.workspace_path,
                display_name: Some(request.name),
            })?;
        Ok(IpcWorkspaceResponse {
            ok: true,
            workspace_id: response.workspace_id,
            name: response.name,
        })
    }

    pub fn workspace_update(
        &self,
        request: WorkspaceUpdateRequest,
    ) -> Result<IpcWorkspaceResponse, TauriIpcError> {
        let response = self
            .workspace_session_service
            .rename_workspace(RenameWorkspaceRequest {
                workspace_id: request.workspace_id,
                name: request.name,
            })?;
        Ok(IpcWorkspaceResponse {
            ok: true,
            workspace_id: response.workspace_id,
            name: response.name,
        })
    }

    pub fn workspace_delete(
        &self,
        request: WorkspaceDeleteRequest,
    ) -> Result<IpcWorkspaceDeleteResponse, TauriIpcError> {
        let response = self
            .workspace_session_service
            .delete_workspace(&request.workspace_id)?;
        Ok(IpcWorkspaceDeleteResponse {
            ok: true,
            workspace_id: response.workspace_id,
            deleted: response.deleted,
        })
    }

    pub fn session_create(
        &self,
        request: SessionCreateRequest,
    ) -> Result<IpcSessionCreateResponse, TauriIpcError> {
        let response = self
            .workspace_session_service
            .create_session(CreateSessionRequest {
                workspace_id: request.workspace_id,
                first_prompt: request.first_prompt,
            })?;
        Ok(IpcSessionCreateResponse {
            ok: true,
            session_id: response.session_id,
            title: response.title,
            round_id: response.round_id,
            message_enqueued: response.message_enqueued,
        })
    }

    pub fn session_list(
        &self,
        request: SessionListRequest,
    ) -> Result<IpcSessionListResponse, TauriIpcError> {
        let sessions = self
            .workspace_session_service
            .list_sessions_by_workspace(&request.workspace_id)?;
        Ok(IpcSessionListResponse {
            ok: true,
            workspace_id: request.workspace_id,
            sessions,
        })
    }

    pub fn session_update(
        &self,
        request: SessionUpdateRequest,
    ) -> Result<IpcSessionUpdateResponse, TauriIpcError> {
        let response = self
            .workspace_session_service
            .rename_session(RenameSessionRequest {
                session_id: request.session_id,
                title: request.title,
            })?;
        Ok(IpcSessionUpdateResponse {
            ok: true,
            session_id: response.session_id,
            title: response.title,
        })
    }

    pub fn session_send_message(
        &self,
        request: SessionSendMessageRequest,
    ) -> Result<IpcSessionSendMessageResponse, TauriIpcError> {
        self.event_bus
            .register_session(&request.session_id)
            .map_err(|error| TauriIpcError::ServiceFailed(error.to_string()))?;
        let response =
            self.workspace_session_service
                .append_user_message(AppendUserMessageRequest {
                    session_id: request.session_id.clone(),
                    content: request.content,
                })?;
        let event = self
            .event_bus
            .publish(EventEnvelope::new(
                EventType::UserMessageReceived,
                &request.session_id,
                None,
                Some(&response.round_id),
                serde_json::json!({"message_enqueued": true}),
            ))
            .map_err(|error| TauriIpcError::ServiceFailed(error.to_string()))?;
        Ok(IpcSessionSendMessageResponse {
            ok: true,
            session_id: response.session_id,
            round_id: response.round_id,
            message_enqueued: true,
            event_id: event.event_id,
        })
    }

    pub fn session_switch_model(
        &self,
        request: SessionSwitchModelRequest,
    ) -> Result<IpcSessionSwitchModelResponse, TauriIpcError> {
        let response =
            self.workspace_session_service
                .change_session_model(ChangeSessionModelRequest {
                    session_id: request.session_id.clone(),
                    model_name: request.model_name,
                    context_limit: self
                        .workspace_session_service
                        .get_session(&request.session_id)?
                        .context_limit,
                })?;
        Ok(IpcSessionSwitchModelResponse {
            ok: true,
            session_id: request.session_id,
            current_model: response.current_model,
            context_limit: response.context_limit,
        })
    }

    pub fn session_switch_approval_mode(
        &self,
        request: SessionSwitchApprovalModeRequest,
    ) -> Result<IpcSessionSwitchApprovalModeResponse, TauriIpcError> {
        let response = self
            .workspace_session_service
            .change_session_approval_mode(ChangeSessionApprovalModeRequest {
                session_id: request.session_id.clone(),
                session_approval_mode: request.target_mode,
            })?;
        Ok(IpcSessionSwitchApprovalModeResponse {
            ok: true,
            session_id: request.session_id,
            session_approval_mode: response.session_approval_mode,
        })
    }

    pub fn session_subscribe_stream(
        &self,
        _request: SessionSubscribeStreamRequest,
    ) -> Result<(), IpcNotImplementedError> {
        Err(IpcNotImplementedError {
            ok: false,
            error_code: "NOT_IMPLEMENTED".to_string(),
            message: "当前阶段仅保留流式订阅 IPC 契约，尚未实现前端订阅。".to_string(),
            retryable: false,
        })
    }
}
