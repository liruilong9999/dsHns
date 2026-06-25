use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;
use dshns_core::{
    config::{AgentConfig, ApprovalMode, ContextConfig},
    event::AgentEvent, message::{Message, Usage},
    tool::{ToolCall, ToolOutcome, ToolStatus}, error::DshnsError,
};
use dshns_deepseek_client::client::DeepSeekClient;
use dshns_deepseek_client::request::ChatRequest;
use dshns_deepseek_client::response::StreamEvent;
use dshns_tools::registry::ToolRegistry;
use dshns_tools::executor::ToolExecutor;
use crate::{context::ContextManager, safety::SafetyGuard, approval::{Approver, ApprovalVerdict}};

pub struct AgentLoop {
    client: Arc<DeepSeekClient>,
    executor: Arc<ToolExecutor>,
    registry: Arc<ToolRegistry>,
    config: AgentConfig,
    context_manager: ContextManager,
    safety_guard: SafetyGuard,
    approver: tokio::sync::Mutex<Approver>,
    pub depth: u8,
    allowed_tools: Option<Vec<String>>,
    system_prompt: String,
}

#[derive(Debug)]
pub struct AgentOutcome {
    pub final_response: String,
    pub messages: Vec<Message>,
    pub usage: Usage,
    pub tool_rounds: u32,
}

impl AgentLoop {
    pub fn new(
        client: Arc<DeepSeekClient>, executor: Arc<ToolExecutor>,
        registry: Arc<ToolRegistry>, config: AgentConfig,
        context_config: ContextConfig, approval_mode: ApprovalMode,
        depth: u8, system_prompt: String,
    ) -> Self {
        Self {
            client, executor, registry, config,
            context_manager: ContextManager::new(context_config),
            safety_guard: SafetyGuard::new(),
            approver: tokio::sync::Mutex::new(Approver::new(approval_mode)),
            depth, allowed_tools: None, system_prompt,
        }
    }

    pub fn set_allowed_tools(&mut self, tools: Vec<String>) { self.allowed_tools = Some(tools); }
    pub async fn approval_mode(&self) -> ApprovalMode { self.approver.lock().await.mode() }
    pub async fn set_approval_mode(&self, mode: ApprovalMode) { self.approver.lock().await.set_mode(mode); }

    pub async fn run(&self, user_input: &str, history: Vec<Message>, event_tx: mpsc::UnboundedSender<AgentEvent>) -> Result<AgentOutcome, DshnsError> {
        let _ = event_tx.send(AgentEvent::UserInput(user_input.into()));
        let mut messages = self.context_manager.build_messages(&self.system_prompt, &history, user_input);
        let mut tool_rounds: u32 = 0;
        let mut total_usage = Usage::default();
        let mut consecutive_failures: u32 = 0;

        loop {
            let api_tools = if let Some(ref allowed) = self.allowed_tools {
                let names = self.registry.get_names();
                let exclude: Vec<&str> = names.iter()
                    .filter(|n| !allowed.contains(n)).map(|s| s.as_str()).collect();
                self.registry.to_api_tools_excluding(&exclude)
            } else {
                self.registry.to_api_tools()
            };

            let req = ChatRequest {
                model: "deepseek-v4-flash".into(), messages: messages.clone(),
                tools: Some(api_tools), stream: true,
                temperature: Some(0.0), max_tokens: Some(8192),
            };

            let mut stream = self.client.chat_stream(&req).await?;
            use futures::StreamExt;
            let mut text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut pending: HashMap<String, (String, String)> = HashMap::new();
            let mut usage = Usage::default();

            while let Some(ev) = stream.next().await {
                match ev {
                    Ok(StreamEvent::TextDelta { delta }) => { text.push_str(&delta); let _ = event_tx.send(AgentEvent::Thinking(delta)); }
                    Ok(StreamEvent::ToolCallStart { id, name }) => { pending.insert(id, (name, String::new())); }
                    Ok(StreamEvent::ToolCallArgDelta { id, delta }) => { if let Some((_, a)) = pending.get_mut(&id) { a.push_str(&delta); } }
                    Ok(StreamEvent::ToolCallComplete { id, name, arguments }) => { tool_calls.push(ToolCall { id: id.clone(), name, arguments }); pending.remove(&id); }
                    Ok(StreamEvent::Finished { usage: u, .. }) => { usage = u; break; }
                    Ok(StreamEvent::Error { message }) => return Err(DshnsError::SseParse(message)),
                    Err(e) => return Err(e),
                }
            }

            // 处理未完成的 tool calls
            for (id, (name, args)) in pending.drain() {
                if let Ok(args_val) = serde_json::from_str(&args) {
                    tool_calls.push(ToolCall { id, name, arguments: args_val });
                }
            }

            total_usage.prompt_tokens += usage.prompt_tokens;
            total_usage.completion_tokens += usage.completion_tokens;
            total_usage.total_tokens += usage.total_tokens;

            // 追加 assistant message
            if tool_calls.is_empty() {
                messages.push(Message::Assistant { content: Some(text.clone()), tool_calls: None });
            } else {
                let refs: Vec<_> = tool_calls.iter().map(|tc| dshns_core::message::ToolCallRef {
                    id: tc.id.clone(), call_type: "function".into(),
                    function: dshns_core::message::FunctionRef { name: tc.name.clone(), arguments: tc.arguments.to_string() },
                }).collect();
                messages.push(Message::Assistant { content: if text.is_empty() { None } else { Some(text.clone()) }, tool_calls: Some(refs) });
            }

            if tool_calls.is_empty() {
                let _ = event_tx.send(AgentEvent::TurnComplete { usage: total_usage.clone(), tool_rounds });
                let _ = event_tx.send(AgentEvent::SessionComplete);
                return Ok(AgentOutcome { final_response: text, messages, usage: total_usage, tool_rounds });
            }

            tool_rounds += 1;
            if tool_rounds >= self.config.max_tool_rounds {
                let _ = event_tx.send(AgentEvent::Error("达到最大工具轮数".into()));
                let _ = event_tx.send(AgentEvent::SessionComplete);
                return Ok(AgentOutcome { final_response: text, messages, usage: total_usage, tool_rounds });
            }

            for call in &tool_calls {
                // 硬限制
                if let Some(blocked) = self.safety_guard.check(call) {
                    let _ = event_tx.send(AgentEvent::ToolBlocked { call: call.clone(), reason: format!("{:?}", blocked.status) });
                    messages.push(Message::Tool { tool_call_id: call.id.clone(), content: blocked.content.clone() });
                    continue;
                }
                // 软审批
                let verdict = self.approver.lock().await.check(call);
                match verdict {
                    ApprovalVerdict::NeedsConfirmation { reason } => {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let _ = event_tx.send(AgentEvent::ToolConfirmationNeeded {
                            call: call.clone(), reason, response_tx: tx,
                        });
                        // 等待用户在 REPL 中回复 y/n
                        match rx.await {
                            Ok(true) => { /* 用户批准，继续执行 */ }
                            Ok(false) | Err(_) => {
                                // 用户拒绝或 channel 关闭
                                let _ = event_tx.send(AgentEvent::ToolExecution {
                                    call_id: call.id.clone(),
                                    status: ToolStatus::Denied,
                                    summary: "用户拒绝".into(),
                                });
                                messages.push(Message::Tool {
                                    tool_call_id: call.id.clone(),
                                    content: "用户拒绝了此操作".into(),
                                });
                                continue;
                            }
                        }
                    }
                    ApprovalVerdict::Allow => {}
                }
                // 执行
                let _ = event_tx.send(AgentEvent::ToolCallStart { id: call.id.clone(), name: call.name.clone() });
                let outcome = self.executor.exec_one(call).await;
                let _ = event_tx.send(AgentEvent::ToolExecution { call_id: call.id.clone(), status: outcome.status.clone(), summary: outcome.content.chars().take(200).collect() });

                match &outcome.status {
                    ToolStatus::Error { .. } | ToolStatus::Timeout => consecutive_failures += 1,
                    ToolStatus::HardBlocked { .. } => {},
                    _ => consecutive_failures = 0,
                }
                if consecutive_failures >= 5 {
                    return Err(DshnsError::ToolLoopStuck { failures: consecutive_failures });
                }
                messages.push(Message::Tool { tool_call_id: call.id.clone(), content: self.format_result(&outcome, call) });
            }
        }
    }

    fn format_result(&self, outcome: &ToolOutcome, call: &ToolCall) -> String {
        match &outcome.status {
            ToolStatus::Success => format!("工具 {} 执行成功:\n{}", call.name, outcome.content),
            ToolStatus::Error { reason } => format!("工具 {} 执行失败: {}", call.name, reason),
            ToolStatus::Timeout => format!("工具 {} 超时，请尝试其他方式", call.name),
            ToolStatus::Denied => "用户拒绝了此操作".into(),
            ToolStatus::HardBlocked { reason } => format!("{} 已被系统禁止: {}", call.name, reason),
        }
    }
}
