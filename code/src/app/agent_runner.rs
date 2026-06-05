//! 智能体单轮执行器。
//!
//! 本模块实现提示词装配、模型调用、工具执行和最终输出回写的单轮状态机。

use crate::domain::tool::{ToolCallRequest, ToolExecutionStatus, ToolResponse};
use crate::infra::config::{AppConfig, ModelGatewayAvailability};
use crate::infra::context_management::{
    CompressionReason, ContextManager, ContextManagerConfig, LongResultBudgetInput,
    LongResultStrategy,
};
use crate::infra::db::SqliteDatabase;
use crate::infra::event_bus::{EventBus, EventEnvelope, EventType};
use crate::infra::prompting::{PromptAssembler, PromptAssemblerConfig, PromptAssemblyInput};
use crate::infra::repository::{MessageRepository, RepositoryError, SessionRepository};
use crate::infra::tool_system::{ToolDispatcher, ToolRuntimeConfig};
use serde_json::Value;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;

/// 模型网关请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelGatewayRequest {
    /// 当前模型名。
    pub model_name: String,
    /// 装配后的提示词。
    pub prompt: String,
}

/// 模型网关响应。
#[derive(Debug, Clone, PartialEq)]
pub enum ModelGatewayResponse {
    /// 模型直接返回最终文本。
    FinalText {
        /// 最终回答内容。
        content: String,
    },
    /// 模型返回工具调用请求。
    ToolCall {
        /// 工具名。
        tool_name: String,
        /// 调用参数。
        arguments: Value,
    },
}

/// 模型网关错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelGatewayError {
    /// 模型请求失败。
    RequestFailed(String),
}

impl Display for ModelGatewayError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ModelGatewayError::RequestFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for ModelGatewayError {}

/// 模型网关抽象。
pub trait ModelGatewayTrait {
    /// 发起模型请求。
    fn complete(
        &self,
        request: ModelGatewayRequest,
    ) -> Result<ModelGatewayResponse, ModelGatewayError>;
}

/// 智能体执行配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRunnerConfig {
    /// 当前工作区根目录。
    pub workspace_root_path: PathBuf,
    /// 技能根目录。
    pub skill_root_path: PathBuf,
    /// 全局 `AGENTS.md` 路径。
    pub global_agents_path: Option<PathBuf>,
    /// 系统提示词。
    pub system_prompt: String,
}

impl AgentRunnerConfig {
    /// 构造默认配置。
    pub fn new(workspace_root_path: PathBuf, skill_root_path: PathBuf) -> Self {
        Self {
            workspace_root_path,
            skill_root_path,
            global_agents_path: None,
            system_prompt: "你是 DeepSeek 专属 Agent，请遵守当前工作区约束。".to_string(),
        }
    }
}

/// 单轮执行请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRoundRequest {
    /// 会话标识。
    pub session_id: String,
    /// 智能体标识。
    pub agent_id: String,
    /// 当前用户输入。
    pub user_input: String,
}

/// 单轮执行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRoundStatus {
    /// 执行完成。
    Completed,
    /// 执行中止。
    Aborted,
}

/// 单轮执行结果。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRoundOutcome {
    /// 最终状态。
    pub status: AgentRoundStatus,
    /// 最终文本输出。
    pub final_text: Option<String>,
    /// 状态历史。
    pub state_history: Vec<String>,
    /// 本轮工具响应列表。
    pub tool_responses: Vec<ToolResponse>,
    /// 首次提示词快照。
    pub prompt_snapshot: String,
    /// 装配告警。
    pub warnings: Vec<String>,
}

/// 单轮执行错误。
#[derive(Debug)]
pub enum AgentRunnerError {
    /// 模型网关不可用。
    ModelUnavailable(String),
    /// 仓储访问失败。
    RepositoryFailed(String),
    /// 模型请求失败。
    ModelRequestFailed(String),
}

impl Display for AgentRunnerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            AgentRunnerError::ModelUnavailable(message) => write!(f, "{message}"),
            AgentRunnerError::RepositoryFailed(message) => write!(f, "{message}"),
            AgentRunnerError::ModelRequestFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for AgentRunnerError {}

impl From<RepositoryError> for AgentRunnerError {
    fn from(value: RepositoryError) -> Self {
        AgentRunnerError::RepositoryFailed(value.to_string())
    }
}

/// 智能体单轮执行器。
pub struct AgentRunner<'a, G: ModelGatewayTrait> {
    database: &'a SqliteDatabase,
    config: &'a AppConfig,
    gateway: G,
    runner_config: AgentRunnerConfig,
    event_bus: Option<EventBus<'a>>,
}

impl<'a, G: ModelGatewayTrait> AgentRunner<'a, G> {
    /// 构造智能体单轮执行器。
    pub fn new(
        database: &'a SqliteDatabase,
        config: &'a AppConfig,
        gateway: G,
        runner_config: AgentRunnerConfig,
    ) -> Self {
        Self {
            database,
            config,
            gateway,
            runner_config,
            event_bus: None,
        }
    }

    /// 注入事件总线。
    pub fn with_event_bus(mut self, event_bus: EventBus<'a>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// 执行一轮从用户输入到最终输出的流程。
    pub fn run_round(
        &self,
        request: AgentRoundRequest,
    ) -> Result<AgentRoundOutcome, AgentRunnerError> {
        if !matches!(
            self.config.model_gateway().availability(),
            ModelGatewayAvailability::Available
        ) {
            return Err(AgentRunnerError::ModelUnavailable(
                self.config
                    .model_gateway()
                    .user_facing_message()
                    .to_string(),
            ));
        }

        let session_repository = SessionRepository::new(self.database.connection());
        let message_repository = MessageRepository::new(self.database.connection());
        let session = session_repository
            .get_by_id(&request.session_id)?
            .ok_or_else(|| {
                AgentRunnerError::RepositoryFailed(format!("会话不存在：{}", request.session_id))
            })?;

        let round_id = session_repository.next_round_id()?;
        self.publish_event(EventEnvelope::new(
            EventType::UserMessageReceived,
            &request.session_id,
            None,
            Some(&round_id),
            serde_json::json!({ "content": request.user_input }),
        ));
        message_repository.create_runtime_message(
            &request.session_id,
            &request.agent_id,
            &round_id,
            "user",
            &request.user_input,
            "plain",
            true,
        )?;

        let prompt_assembler = PromptAssembler::new(PromptAssemblerConfig {
            global_agents_path: self.runner_config.global_agents_path.clone(),
            workspace_root_path: self.runner_config.workspace_root_path.clone(),
            skill_root_path: self.runner_config.skill_root_path.clone(),
            system_prompt: self.runner_config.system_prompt.clone(),
        });
        let context_manager = ContextManager::new(self.database, ContextManagerConfig::default());
        let tool_dispatcher = &mut ToolDispatcher::new(ToolRuntimeConfig::new(
            self.runner_config.workspace_root_path.clone(),
            self.runner_config.skill_root_path.clone(),
        ));

        let mut state_history = vec!["Idle".to_string(), "PreparingContext".to_string()];
        let mut warnings = Vec::new();
        let mut tool_responses = Vec::new();
        let mut current_messages = message_repository.list_by_session_id(&request.session_id)?;

        let first_assembly = prompt_assembler
            .assemble(
                tool_dispatcher.registry(),
                PromptAssemblyInput {
                    messages: &current_messages,
                    current_user_input: &request.user_input,
                    compression_summary: None,
                    context_limit: session.context_limit as u32,
                    expected_output_tokens: 4_096,
                },
            )
            .map_err(AgentRunnerError::ModelRequestFailed)?;
        warnings.extend(first_assembly.warnings.clone());

        let (first_prompt, prompt_snapshot) = if first_assembly.requires_compression {
            let compression = context_manager.compress_session_context(
                &request.session_id,
                &request.agent_id,
                first_assembly.estimated_tokens,
                CompressionReason::OverLimit,
            )?;
            let recomposed = prompt_assembler
                .assemble(
                    tool_dispatcher.registry(),
                    PromptAssemblyInput {
                        messages: &compression.kept_messages,
                        current_user_input: &request.user_input,
                        compression_summary: Some(&compression.summary_text),
                        context_limit: session.context_limit as u32,
                        expected_output_tokens: 4_096,
                    },
                )
                .map_err(AgentRunnerError::ModelRequestFailed)?;
            warnings.extend(recomposed.warnings.clone());
            if recomposed.requires_compression {
                return Err(AgentRunnerError::ModelRequestFailed(
                    "压缩后仍超上下文上限，无法继续发起模型请求。".to_string(),
                ));
            }
            (recomposed.prompt.clone(), recomposed.prompt)
        } else {
            (first_assembly.prompt.clone(), first_assembly.prompt)
        };

        state_history.push("CallingModel".to_string());
        self.publish_event(EventEnvelope::new(
            EventType::ModelThinkingStarted,
            &request.session_id,
            Some(&request.agent_id),
            Some(&round_id),
            serde_json::json!({ "stage": "before_first_model_call" }),
        ));
        match self.gateway.complete(ModelGatewayRequest {
            model_name: session.current_model.clone(),
            prompt: first_prompt,
        }) {
            Ok(ModelGatewayResponse::FinalText { content }) => {
                self.publish_event(EventEnvelope::new(
                    EventType::AssistantOutputDelta,
                    &request.session_id,
                    Some(&request.agent_id),
                    Some(&round_id),
                    serde_json::json!({ "delta": content.clone() }),
                ));
                message_repository.create_runtime_message(
                    &request.session_id,
                    &request.agent_id,
                    &round_id,
                    "assistant",
                    &content,
                    "plain",
                    true,
                )?;
                self.publish_event(EventEnvelope::new(
                    EventType::AssistantOutputCompleted,
                    &request.session_id,
                    Some(&request.agent_id),
                    Some(&round_id),
                    serde_json::json!({ "content": content.clone() }),
                ));
                state_history.push("Completed".to_string());
                Ok(AgentRoundOutcome {
                    status: AgentRoundStatus::Completed,
                    final_text: Some(content),
                    state_history,
                    tool_responses,
                    prompt_snapshot,
                    warnings,
                })
            }
            Ok(ModelGatewayResponse::ToolCall {
                tool_name,
                arguments,
            }) => {
                state_history.push("DispatchingTool".to_string());
                self.publish_event(EventEnvelope::new(
                    EventType::ToolStarted,
                    &request.session_id,
                    Some(&request.agent_id),
                    Some(&round_id),
                    serde_json::json!({ "tool_name": tool_name.clone() }),
                ));
                let tool_response = tool_dispatcher.execute(
                    ToolCallRequest::new(
                        &tool_name,
                        &request.session_id,
                        &request.agent_id,
                        &round_id,
                        arguments,
                    ),
                    parse_session_approval_mode(&session.session_approval_mode),
                );
                tool_responses.push(tool_response.clone());
                self.publish_event(EventEnvelope::new(
                    EventType::ToolFinished,
                    &request.session_id,
                    Some(&request.agent_id),
                    Some(&round_id),
                    serde_json::json!({
                        "tool_name": tool_response.tool_name.clone(),
                        "status": format!("{:?}", tool_response.status)
                    }),
                ));

                let raw_tool_content = if tool_response.status == ToolExecutionStatus::Success {
                    serde_json::to_string_pretty(&tool_response.result_payload)
                        .unwrap_or_else(|_| "工具结果序列化失败。".to_string())
                } else {
                    tool_response
                        .message
                        .clone()
                        .unwrap_or_else(|| "工具执行失败。".to_string())
                };
                let long_result_budget =
                    context_manager.handle_long_result(LongResultBudgetInput {
                        content: raw_tool_content,
                        is_failure: tool_response.status != ToolExecutionStatus::Success,
                        context_tokens_before_result: context_manager
                            .estimate_tokens(&prompt_snapshot),
                        max_context: session.context_limit as u32,
                        tool_tokens: 0,
                        skill_tokens: 0,
                        expected_output_tokens: 4_096,
                        model_name: session.current_model.clone(),
                        summary_agent_available: false,
                    });
                let mut second_compression_summary = None;
                if long_result_budget.strategy == LongResultStrategy::DirectAfterCompression {
                    let compression = context_manager.compress_session_context(
                        &request.session_id,
                        &request.agent_id,
                        context_manager.estimate_tokens(&prompt_snapshot),
                        CompressionReason::OverLimit,
                    )?;
                    second_compression_summary = Some(compression.summary_text);
                }
                let tool_content = long_result_budget.content;
                message_repository.create_runtime_message(
                    &request.session_id,
                    &request.agent_id,
                    &round_id,
                    "tool",
                    &tool_content,
                    "plain",
                    true,
                )?;

                state_history.push("ApplyingResult".to_string());
                current_messages = message_repository.list_by_session_id(&request.session_id)?;
                let second_assembly = prompt_assembler
                    .assemble(
                        tool_dispatcher.registry(),
                        PromptAssemblyInput {
                            messages: &current_messages,
                            current_user_input: "",
                            compression_summary: second_compression_summary.as_deref(),
                            context_limit: session.context_limit as u32,
                            expected_output_tokens: 4_096,
                        },
                    )
                    .map_err(AgentRunnerError::ModelRequestFailed)?;
                warnings.extend(second_assembly.warnings);

                state_history.push("CallingModel".to_string());
                self.publish_event(EventEnvelope::new(
                    EventType::ModelThinkingStarted,
                    &request.session_id,
                    Some(&request.agent_id),
                    Some(&round_id),
                    serde_json::json!({ "stage": "after_tool_call" }),
                ));
                match self.gateway.complete(ModelGatewayRequest {
                    model_name: session.current_model.clone(),
                    prompt: second_assembly.prompt,
                }) {
                    Ok(ModelGatewayResponse::FinalText { content }) => {
                        self.publish_event(EventEnvelope::new(
                            EventType::AssistantOutputDelta,
                            &request.session_id,
                            Some(&request.agent_id),
                            Some(&round_id),
                            serde_json::json!({ "delta": content.clone() }),
                        ));
                        message_repository.create_runtime_message(
                            &request.session_id,
                            &request.agent_id,
                            &round_id,
                            "assistant",
                            &content,
                            "plain",
                            true,
                        )?;
                        self.publish_event(EventEnvelope::new(
                            EventType::AssistantOutputCompleted,
                            &request.session_id,
                            Some(&request.agent_id),
                            Some(&round_id),
                            serde_json::json!({ "content": content.clone() }),
                        ));
                        state_history.push("Completed".to_string());
                        Ok(AgentRoundOutcome {
                            status: AgentRoundStatus::Completed,
                            final_text: Some(content),
                            state_history,
                            tool_responses,
                            prompt_snapshot,
                            warnings,
                        })
                    }
                    Ok(ModelGatewayResponse::ToolCall { .. }) => {
                        Err(AgentRunnerError::ModelRequestFailed(
                            "当前阶段单轮执行器暂不支持连续多次工具调用。".to_string(),
                        ))
                    }
                    Err(error) => Err(AgentRunnerError::ModelRequestFailed(error.to_string())),
                }
            }
            Err(error) => Err(AgentRunnerError::ModelRequestFailed(error.to_string())),
        }
    }

    fn publish_event(&self, event: EventEnvelope) {
        if let Some(event_bus) = &self.event_bus {
            let _ = event_bus.publish(event);
        }
    }
}

/// 把数据库中的审批模式字符串转换为领域枚举。
fn parse_session_approval_mode(value: &str) -> crate::domain::tool::SessionApprovalMode {
    match value {
        "auto" => crate::domain::tool::SessionApprovalMode::Auto,
        "allow_all" => crate::domain::tool::SessionApprovalMode::AllowAll,
        _ => crate::domain::tool::SessionApprovalMode::Ask,
    }
}
