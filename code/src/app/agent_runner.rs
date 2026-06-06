//! 智能体单轮执行器。
//!
//! 本模块实现提示词装配、模型调用、工具执行和最终输出回写的单轮状态机。

use crate::domain::tool::{SessionApprovalMode, ToolCallRequest, ToolExecutionStatus, ToolResponse};
use crate::infra::config::{AppConfig, ModelGatewayAvailability};
use crate::infra::context_management::{
    CompressionReason, ContextManager, ContextManagerConfig, LongResultBudgetInput,
    LongResultStrategy,
};
use crate::infra::db::SqliteDatabase;
use crate::infra::event_bus::{EventBus, EventEnvelope, EventType};
use crate::infra::metrics::{MetricsCollector, SessionMetricInput};
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
    /// 结构化消息列表。
    pub messages: Vec<Value>,
    /// 工具定义列表。
    pub tools: Vec<Value>,
    /// 最大输出预算。
    pub max_tokens: u32,
}

/// 模型网关响应。
#[derive(Debug, Clone, PartialEq)]
pub enum ModelGatewayResponse {
    /// 模型直接返回最终文本。
    FinalText {
        /// 最终回答内容。
        content: String,
        /// 模型思考内容。
        reasoning_content: Option<String>,
    },
    /// 模型返回工具调用请求。
    ToolCall {
        /// 工具名。
        tool_name: String,
        /// 调用参数。
        arguments: Value,
        /// 工具调用标识。
        tool_call_id: String,
        /// 助手在发起工具调用时附带的内容。
        assistant_content: Option<String>,
        /// 模型思考内容。
        reasoning_content: Option<String>,
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
    /// 如果输入已提前写入会话，则跳过再次落盘。
    pub input_already_persisted: bool,
    /// 已存在的轮次标识。
    pub existing_round_id: Option<String>,
    /// 可选审批模式覆盖，用于本地确认继续执行等场景。
    pub approval_mode_override: Option<SessionApprovalMode>,
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

        let round_id = request
            .existing_round_id
            .clone()
            .unwrap_or(session_repository.next_round_id()?);
        if !request.input_already_persisted {
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
        }

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
        let mut tool_responses: Vec<ToolResponse> = Vec::new();
        let current_messages = message_repository.list_by_session_id(&request.session_id)?;

        let first_assembly = prompt_assembler
            .assemble(
                tool_dispatcher.registry(),
                PromptAssemblyInput {
                    messages: &current_messages,
                    current_user_input: if request.input_already_persisted {
                        ""
                    } else {
                        &request.user_input
                    },
                    compression_summary: None,
                    context_limit: session.context_limit as u32,
                    expected_output_tokens: 4_096,
                },
            )
            .map_err(AgentRunnerError::ModelRequestFailed)?;
        warnings.extend(first_assembly.warnings.clone());

        let (prompt_snapshot, first_gateway_messages) = if first_assembly.requires_compression {
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
            (recomposed.prompt, recomposed.gateway_messages)
        } else {
            (first_assembly.prompt, first_assembly.gateway_messages.clone())
        };

        let mut gateway_messages = first_gateway_messages.clone();
        loop {
            state_history.push("CallingModel".to_string());
            self.publish_event(EventEnvelope::new(
                EventType::ModelThinkingStarted,
                &request.session_id,
                Some(&request.agent_id),
                Some(&round_id),
                serde_json::json!({ "stage": "model_call" }),
            ));
            match self.gateway.complete(ModelGatewayRequest {
                model_name: session.current_model.clone(),
                prompt: prompt_snapshot.clone(),
                messages: gateway_messages.clone(),
                tools: tool_dispatcher.registry().to_gateway_tools(),
                max_tokens: 4_096,
            }) {
                Ok(ModelGatewayResponse::FinalText {
                    content,
                    reasoning_content,
                }) => {
                    if let Some(reasoning_content) = reasoning_content {
                        self.publish_event(EventEnvelope::new(
                            EventType::ModelThinkingStarted,
                            &request.session_id,
                            Some(&request.agent_id),
                            Some(&round_id),
                            serde_json::json!({
                                "stage": "reasoning",
                                "reasoning_content": reasoning_content
                            }),
                        ));
                    }
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
                    let tool_success_count = tool_responses
                        .iter()
                        .filter(|response| response.status == ToolExecutionStatus::Success)
                        .count() as i64;
                    let tool_failure_count = tool_responses
                        .iter()
                        .filter(|response| response.status != ToolExecutionStatus::Success)
                        .count() as i64;
                    let _ = MetricsCollector::new(self.database)
                        .with_event_bus(
                            self.event_bus
                                .clone()
                                .unwrap_or_else(|| EventBus::new(self.database)),
                        )
                        .record_snapshot(SessionMetricInput {
                            session_id: request.session_id.clone(),
                            agent_id: Some(request.agent_id.clone()),
                            input_tokens: context_manager.estimate_tokens(&prompt_snapshot) as i64,
                            output_tokens: context_manager.estimate_tokens(&content) as i64,
                            cache_hit_rate: 0.0,
                            remaining_context: (session.context_limit
                                - context_manager.estimate_tokens(&prompt_snapshot) as i64
                                - 4_096)
                                .max(0),
                            tool_success_count,
                            tool_failure_count,
                            active_tool_calls: 0,
                        });
                    state_history.push("Completed".to_string());
                    return Ok(AgentRoundOutcome {
                        status: AgentRoundStatus::Completed,
                        final_text: Some(content),
                        state_history,
                        tool_responses,
                        prompt_snapshot,
                        warnings,
                    });
                }
                Ok(ModelGatewayResponse::ToolCall {
                    tool_name,
                    arguments,
                    tool_call_id,
                    assistant_content,
                    reasoning_content,
                }) => {
                    if let Some(reasoning_content) = reasoning_content.clone() {
                        self.publish_event(EventEnvelope::new(
                            EventType::ModelThinkingStarted,
                            &request.session_id,
                            Some(&request.agent_id),
                            Some(&round_id),
                            serde_json::json!({
                                "stage": "reasoning",
                                "reasoning_content": reasoning_content
                            }),
                        ));
                    }
                    state_history.push("DispatchingTool".to_string());
                    self.publish_event(EventEnvelope::new(
                        EventType::ToolStarted,
                        &request.session_id,
                        Some(&request.agent_id),
                        Some(&round_id),
                        serde_json::json!({ "tool_name": tool_name.clone() }),
                    ));
                    let tool_arguments = arguments.clone();
                    let tool_response = tool_dispatcher.execute(
                        ToolCallRequest::new(
                            &tool_name,
                            &request.session_id,
                            &request.agent_id,
                            &round_id,
                            arguments,
                        ),
                        request
                            .approval_mode_override
                            .unwrap_or_else(|| parse_session_approval_mode(&session.session_approval_mode)),
                    );
                    tool_responses.push(tool_response.clone());
                    self.publish_event(EventEnvelope::new(
                        EventType::ToolFinished,
                        &request.session_id,
                        Some(&request.agent_id),
                        Some(&round_id),
                        serde_json::json!({
                            "tool_name": tool_response.tool_name.clone(),
                            "status": format!("{:?}", tool_response.status),
                            "error_code": tool_response.error_code.clone(),
                            "message": tool_response.message.clone()
                        }),
                    ));

                    if tool_response.error_code.as_deref() == Some("APPROVAL_REQUIRED") {
                        state_history.push("Aborted".to_string());
                        return Ok(AgentRoundOutcome {
                            status: AgentRoundStatus::Aborted,
                            final_text: None,
                            state_history,
                            tool_responses,
                            prompt_snapshot,
                            warnings,
                        });
                    }

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
                    if let Some(summary) = second_compression_summary.as_deref() {
                        gateway_messages.push(serde_json::json!({
                            "role": "system",
                            "content": summary
                        }));
                    }
                    gateway_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": assistant_content,
                        "reasoning_content": reasoning_content,
                        "tool_calls": [{
                            "id": tool_call_id,
                            "type": "function",
                            "function": {
                                "name": tool_name,
                                "arguments": serde_json::to_string(&tool_arguments).unwrap_or_else(|_| "{}".to_string())
                            }
                        }]
                    }));
                    gateway_messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": tool_content
                    }));
                }
                Err(error) => return Err(AgentRunnerError::ModelRequestFailed(error.to_string())),
            }
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
