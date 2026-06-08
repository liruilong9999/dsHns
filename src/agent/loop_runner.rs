//! Agent 循环执行器实现。

use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::agent::context::ConversationContextBuilder;
use crate::config::settings::Settings;
use crate::domain::{Message, Session, ToolFailureType};
use crate::ipc::bus::EventBus;
use crate::llm::client::LlmClient;
use crate::session::manager::SessionManager;
use crate::tools::executor::ToolExecutor;
use crate::tools::registry::ToolRegistry;

/// 单轮执行结果。
pub struct TurnOutcome {
    /// 最终回复内容。
    pub final_message: String,
    /// 输入 Token 数。
    pub input_tokens: usize,
    /// 输出 Token 数。
    pub output_tokens: usize,
    /// 是否触发了压缩。
    pub compacted: bool,
}

/// Agent 循环执行器。
pub struct AgentLoopRunner {
    /// 模型客户端。
    llm_client: Arc<LlmClient>,
    /// 工具执行器。
    tool_executor: Arc<ToolExecutor>,
    /// 工具注册表。
    tool_registry: Arc<ToolRegistry>,
    /// 会话管理器。
    session_manager: Arc<SessionManager>,
    /// 运行配置。
    settings: Settings,
    /// 事件总线。
    event_bus: Arc<EventBus>,
}

impl AgentLoopRunner {
    /// 创建 Agent 循环执行器。
    pub fn new(
        llm_client: Arc<LlmClient>,
        tool_executor: Arc<ToolExecutor>,
        tool_registry: Arc<ToolRegistry>,
        session_manager: Arc<SessionManager>,
        settings: Settings,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            llm_client,
            tool_executor,
            tool_registry,
            session_manager,
            settings,
            event_bus,
        }
    }

    /// 执行单轮会话。
    pub async fn run_turn(
        &self,
        session: &Session,
        history: &mut Vec<Message>,
        user_input: &str,
    ) -> Result<TurnOutcome> {
        history.push(Message::user(user_input));

        let context_builder = ConversationContextBuilder::new(model_context_window(&session.model));
        let mut invalid_arg_failures = 0usize;
        let mut tool_failures = 0usize;
        let mut total_tool_calls = 0usize;

        for _ in 0..self.settings.max_rounds {
            let round_no = session.round + 1;
            let context = context_builder.build(&session.system_prompt, history);
            if let Some(summary) = &context.working_memory {
                self.session_manager.save_working_memory(
                    session,
                    summary,
                    context.estimated_tokens_before as i64,
                    context.estimated_tokens_after as i64,
                )?;
                self.event_bus
                    .emit_working_memory(&session.id, round_no, summary)?;
            }

            let model_response = self
                .llm_client
                .chat_completion(
                    &session.model,
                    &context.messages,
                    &self.tool_registry.model_tools(),
                    session.stream_output,
                )
                .await?;

            if !model_response.tool_calls.is_empty() {
                history.push(Message::assistant(
                    model_response.content,
                    model_response.tool_calls.clone(),
                    model_response.reasoning_content.clone(),
                ));

                for tool_call in &model_response.tool_calls {
                    total_tool_calls += 1;
                    if total_tool_calls > self.settings.tool_call_limit {
                        return Err(anyhow!(
                            "单轮工具调用次数超过上限 {}",
                            self.settings.tool_call_limit
                        ));
                    }

                    self.event_bus.emit_tool_status(
                        &session.id,
                        round_no,
                        &tool_call.id,
                        &tool_call.function.name,
                        "running",
                        None,
                    )?;

                    let receipt = self
                        .tool_executor
                        .execute(&session.id, round_no, &session.session_dir, tool_call)
                        .await;

                    self.event_bus.emit_tool_status(
                        &session.id,
                        round_no,
                        &tool_call.id,
                        &tool_call.function.name,
                        if receipt.success { "done" } else { "failed" },
                        Some(receipt.success),
                    )?;

                    if !receipt.success {
                        match receipt.failure_type {
                            Some(ToolFailureType::InvalidArgs) => invalid_arg_failures += 1,
                            _ => tool_failures += 1,
                        }
                    }

                    let tool_content = if receipt.success {
                        receipt.projection_content
                    } else {
                        format!("工具执行失败：{}", receipt.error_message)
                    };
                    history.push(Message::tool(&receipt.tool_call_id, tool_content));
                }

                if invalid_arg_failures >= self.settings.invalid_arg_retry_limit {
                    return Err(anyhow!(
                        "连续非法工具参数次数达到上限 {}，当前轮执行终止",
                        self.settings.invalid_arg_retry_limit
                    ));
                }
                if tool_failures >= self.settings.tool_failure_limit {
                    return Err(anyhow!(
                        "工具连续失败次数达到上限 {}，当前轮执行终止",
                        self.settings.tool_failure_limit
                    ));
                }

                continue;
            }

            history.push(Message::assistant(
                model_response.content.clone(),
                Vec::new(),
                model_response.reasoning_content,
            ));
            self.event_bus
                .emit_stream_chunk(&session.id, round_no, &model_response.content)?;
            let remaining_context = model_context_window(&session.model).saturating_sub(
                model_response
                    .input_tokens
                    .max(context.estimated_tokens_after),
            );
            self.event_bus.emit_token_usage(
                &session.id,
                round_no,
                model_response
                    .input_tokens
                    .max(context.estimated_tokens_before),
                model_response.output_tokens,
                0.0,
                remaining_context,
            )?;
            return Ok(TurnOutcome {
                final_message: model_response.content,
                input_tokens: model_response
                    .input_tokens
                    .max(context.estimated_tokens_before),
                output_tokens: model_response.output_tokens,
                compacted: context.compacted || model_response.stream_fallback,
            });
        }

        Err(anyhow!(
            "超过最大循环轮数 {}，任务被终止",
            self.settings.max_rounds
        ))
    }
}

fn model_context_window(model: &str) -> usize {
    if model.contains("[1m]") {
        1_000_000
    } else {
        256_000
    }
}
