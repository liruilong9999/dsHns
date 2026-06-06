//! CLI 应用层。
//!
//! 本模块负责解析普通输入与斜杠命令，并把请求路由到目录与会话服务。

use crate::app::workspace_session_service::{
    AppendUserMessageRequest, ChangeSessionApprovalModeRequest, ChangeSessionModelRequest,
    CommandAuditRequest, CreateSessionRequest, EnsureWorkspaceRequest, SessionListItem,
    WorkspaceSessionService, WorkspaceSessionServiceError,
};
use crate::infra::config::AppConfig;
use crate::infra::db::SqliteDatabase;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// CLI 显示状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliDisplayState {
    /// 模型思考状态。
    Thinking,
    /// 工具执行中状态。
    ToolRunning,
    /// 工具执行成功状态。
    ToolSuccess,
    /// 工具执行失败状态。
    ToolFailure,
    /// 普通回答状态。
    Answer,
}

impl CliDisplayState {
    /// 返回显示颜色名。
    pub fn color_name(&self) -> &'static str {
        match self {
            CliDisplayState::Thinking => "orange",
            CliDisplayState::ToolRunning => "white",
            CliDisplayState::ToolSuccess => "green",
            CliDisplayState::ToolFailure => "red",
            CliDisplayState::Answer => "white",
        }
    }

    /// 返回前缀标签。
    pub fn prefix_label(&self) -> &'static str {
        match self {
            CliDisplayState::Thinking => "[思考]",
            CliDisplayState::ToolRunning => "[工具执行中]",
            CliDisplayState::ToolSuccess => "[工具成功]",
            CliDisplayState::ToolFailure => "[工具失败]",
            CliDisplayState::Answer => "[回答]",
        }
    }
}

/// CLI 响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliResponse {
    /// 普通文本已进入当前会话处理链路。
    TextAccepted {
        /// 当前会话标识。
        session_id: String,
        /// 当前轮次标识。
        round_id: String,
        /// 是否为首次创建会话。
        created_new_session: bool,
        /// 显示状态。
        display_state: CliDisplayState,
    },
    /// 返回模型列表。
    ModelsListed {
        /// 可用模型列表。
        models: Vec<String>,
    },
    /// 返回当前目录会话列表。
    SessionsListed {
        /// 当前目录标识。
        workspace_id: String,
        /// 会话列表。
        sessions: Vec<SessionListItem>,
    },
    /// 会话模型切换结果。
    ModelChanged {
        /// 会话标识。
        session_id: String,
        /// 当前模型名。
        current_model: String,
        /// 当前上下文上限。
        context_limit: i64,
    },
    /// 会话审批模式切换结果。
    ModeChanged {
        /// 会话标识。
        session_id: String,
        /// 新的审批模式。
        session_approval_mode: String,
    },
    /// 退出 CLI。
    Quit {
        /// 固定为 `true`。
        quit: bool,
    },
}

/// CLI 错误。
#[derive(Debug)]
pub enum CliError {
    /// 输入不合法。
    ValidationFailed(String),
    /// 服务层调用失败。
    ServiceFailed(String),
}

impl Display for CliError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CliError::ValidationFailed(message) => write!(f, "{message}"),
            CliError::ServiceFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for CliError {}

impl From<WorkspaceSessionServiceError> for CliError {
    fn from(value: WorkspaceSessionServiceError) -> Self {
        CliError::ServiceFailed(value.to_string())
    }
}

/// CLI 应用。
pub struct CliApplication<'a> {
    /// 目录与会话服务。
    workspace_session_service: WorkspaceSessionService<'a>,
    /// 应用配置。
    config: &'a AppConfig,
    /// 当前工作区根目录路径。
    workspace_root_path: String,
    /// 当前工作区标识。
    active_workspace_id: Option<String>,
    /// 当前活跃会话标识。
    active_session_id: Option<String>,
}

impl<'a> CliApplication<'a> {
    /// 构造 CLI 应用。
    pub fn new(
        database: &'a SqliteDatabase,
        config: &'a AppConfig,
        workspace_root_path: String,
    ) -> Self {
        Self {
            workspace_session_service: WorkspaceSessionService::new(database, config),
            config,
            workspace_root_path,
            active_workspace_id: None,
            active_session_id: None,
        }
    }

    /// 处理单条输入。
    pub fn handle_input(&mut self, input: &str) -> Result<CliResponse, CliError> {
        let trimmed_input = input.trim();
        if trimmed_input.is_empty() {
            return Err(CliError::ValidationFailed("输入内容不能为空。".to_string()));
        }

        if trimmed_input.starts_with('/') {
            return self.handle_command(trimmed_input);
        }

        self.handle_plain_text(trimmed_input)
    }

    /// 处理普通文本输入。
    fn handle_plain_text(&mut self, input: &str) -> Result<CliResponse, CliError> {
        let workspace_id = self.ensure_active_workspace()?;

        match self.active_session_id.clone() {
            None => {
                let created_session =
                    self.workspace_session_service
                        .create_session(CreateSessionRequest {
                            workspace_id,
                            first_prompt: input.to_string(),
                        })?;
                self.active_session_id = Some(created_session.session_id.clone());

                Ok(CliResponse::TextAccepted {
                    session_id: created_session.session_id,
                    round_id: created_session.round_id,
                    created_new_session: true,
                    display_state: CliDisplayState::Answer,
                })
            }
            Some(active_session_id) => {
                let appended_message = self.workspace_session_service.append_user_message(
                    AppendUserMessageRequest {
                        session_id: active_session_id.clone(),
                        content: input.to_string(),
                    },
                )?;

                Ok(CliResponse::TextAccepted {
                    session_id: active_session_id,
                    round_id: appended_message.round_id,
                    created_new_session: false,
                    display_state: CliDisplayState::Answer,
                })
            }
        }
    }

    /// 处理斜杠命令。
    fn handle_command(&mut self, command_text: &str) -> Result<CliResponse, CliError> {
        self.audit_command_if_possible(command_text)?;

        let parts = command_text.split_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            ["/models"] => Ok(CliResponse::ModelsListed {
                models: self.config.available_model_names(),
            }),
            ["/model", "check", model_name] => self.handle_model_check(model_name),
            ["/mode"] => self.handle_mode_cycle(),
            ["/mode", target_mode] => self.handle_mode_explicit(target_mode),
            ["/sessions"] => self.handle_sessions_list(),
            ["/quit"] => Ok(CliResponse::Quit { quit: true }),
            _ => Err(CliError::ValidationFailed(format!(
                "不支持的命令：{command_text}"
            ))),
        }
    }

    /// 处理 `/model check`。
    fn handle_model_check(&mut self, model_name: &str) -> Result<CliResponse, CliError> {
        let session_id = self.require_active_session_id()?;
        let context_limit = self
            .config
            .context_limit_for_model(model_name)
            .ok_or_else(|| CliError::ValidationFailed(format!("模型不存在：{model_name}")))?;

        let changed =
            self.workspace_session_service
                .change_session_model(ChangeSessionModelRequest {
                    session_id: session_id.clone(),
                    model_name: model_name.to_string(),
                    context_limit: i64::from(context_limit),
                })?;

        Ok(CliResponse::ModelChanged {
            session_id,
            current_model: changed.current_model,
            context_limit: changed.context_limit,
        })
    }

    /// 处理无参 `/mode` 循环切换。
    fn handle_mode_cycle(&mut self) -> Result<CliResponse, CliError> {
        let session_id = self.require_active_session_id()?;
        let current_session = self.workspace_session_service.get_session(&session_id)?;
        let next_mode = match current_session.session_approval_mode.as_str() {
            "ask" => "auto",
            "auto" => "allow_all",
            _ => "ask",
        };

        self.handle_mode_change(session_id, next_mode)
    }

    /// 处理显式 `/mode xxx`。
    fn handle_mode_explicit(&mut self, target_mode: &str) -> Result<CliResponse, CliError> {
        let session_id = self.require_active_session_id()?;
        match target_mode {
            "ask" | "auto" | "allow_all" => self.handle_mode_change(session_id, target_mode),
            _ => Err(CliError::ValidationFailed(format!(
                "审批模式不存在：{target_mode}"
            ))),
        }
    }

    /// 执行审批模式切换。
    fn handle_mode_change(
        &mut self,
        session_id: String,
        target_mode: &str,
    ) -> Result<CliResponse, CliError> {
        let changed = self
            .workspace_session_service
            .change_session_approval_mode(ChangeSessionApprovalModeRequest {
                session_id: session_id.clone(),
                session_approval_mode: target_mode.to_string(),
            })?;

        Ok(CliResponse::ModeChanged {
            session_id,
            session_approval_mode: changed.session_approval_mode,
        })
    }

    /// 处理 `/sessions`。
    fn handle_sessions_list(&mut self) -> Result<CliResponse, CliError> {
        let workspace_id = self.ensure_active_workspace()?;
        let sessions = self
            .workspace_session_service
            .list_sessions_by_workspace(&workspace_id)?;

        Ok(CliResponse::SessionsListed {
            workspace_id,
            sessions,
        })
    }

    /// 在可能的情况下记录命令审计。
    fn audit_command_if_possible(&mut self, command_text: &str) -> Result<(), CliError> {
        if let Some(session_id) = self.active_session_id.clone() {
            self.workspace_session_service
                .record_command_audit(CommandAuditRequest {
                    session_id,
                    command_text: command_text.to_string(),
                })?;
        }

        Ok(())
    }

    /// 确保当前目录已绑定工作区。
    fn ensure_active_workspace(&mut self) -> Result<String, CliError> {
        if let Some(workspace_id) = self.active_workspace_id.clone() {
            return Ok(workspace_id);
        }

        let workspace =
            self.workspace_session_service
                .ensure_workspace(EnsureWorkspaceRequest {
                    root_path: self.workspace_root_path.clone(),
                    display_name: None,
                })?;
        self.active_workspace_id = Some(workspace.workspace_id.clone());
        Ok(workspace.workspace_id)
    }

    /// 获取当前活跃会话标识。
    fn require_active_session_id(&self) -> Result<String, CliError> {
        self.active_session_id.clone().ok_or_else(|| {
            CliError::ValidationFailed("当前没有活跃会话，请先输入普通文本创建会话。".to_string())
        })
    }

    /// 返回当前活跃会话标识。
    pub fn current_session_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    /// 返回当前活跃工作区标识。
    pub fn current_workspace_id(&self) -> Option<&str> {
        self.active_workspace_id.as_deref()
    }

    /// 返回当前活跃会话的审批模式。
    pub fn current_approval_mode(&self) -> Result<Option<String>, CliError> {
        let Some(session_id) = self.active_session_id.as_deref() else {
            return Ok(None);
        };
        Ok(Some(
            self.workspace_session_service
                .get_session(session_id)?
                .session_approval_mode,
        ))
    }
}
