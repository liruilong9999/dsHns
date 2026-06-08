//! 上下文预算与压缩实现。

use crate::domain::Message;

/// 上下文构建结果。
pub struct ContextBuildResult {
    /// 发送给模型的消息列表。
    pub messages: Vec<Message>,
    /// 是否发生了压缩。
    pub compacted: bool,
    /// 估算压缩前 Token 数。
    pub estimated_tokens_before: usize,
    /// 估算压缩后 Token 数。
    pub estimated_tokens_after: usize,
    /// 自动生成的工作记忆摘要。
    pub working_memory: Option<String>,
}

/// 上下文构建器。
pub struct ConversationContextBuilder {
    /// 上下文窗口上限。
    max_context_tokens: usize,
}

impl ConversationContextBuilder {
    /// 创建上下文构建器。
    pub fn new(max_context_tokens: usize) -> Self {
        Self { max_context_tokens }
    }

    /// 组装会话上下文，必要时自动压缩较早历史。
    pub fn build(&self, system_prompt: &str, history: &[Message]) -> ContextBuildResult {
        let estimated_tokens_before = estimate_tokens(system_prompt)
            + history.iter().map(estimate_message_tokens).sum::<usize>();
        let budget = self.max_context_tokens.saturating_mul(8) / 10;

        if estimated_tokens_before <= budget {
            let mut messages = vec![Message::system(system_prompt)];
            messages.extend(history.iter().cloned());
            return ContextBuildResult {
                estimated_tokens_before,
                estimated_tokens_after: estimated_tokens_before,
                messages,
                compacted: false,
                working_memory: None,
            };
        }

        let mut kept = Vec::new();
        let mut kept_tokens = estimate_tokens(system_prompt);
        for message in history.iter().rev() {
            let tokens = estimate_message_tokens(message);
            if kept_tokens + tokens > budget && !kept.is_empty() {
                break;
            }
            kept.push(message.clone());
            kept_tokens += tokens;
        }
        kept.reverse();

        let dropped_count = history.len().saturating_sub(kept.len());
        let summary = if dropped_count > 0 {
            let lines = history
                .iter()
                .take(dropped_count)
                .map(|message| format!("- {:?}: {}", message.role, truncate(&message.content, 80)))
                .collect::<Vec<_>>()
                .join("\n");
            Some(format!("历史摘要（自动压缩）：\n{}", lines))
        } else {
            None
        };

        let mut messages = vec![Message::system(system_prompt)];
        if let Some(summary_text) = &summary {
            messages.push(Message::system(summary_text.clone()));
        }
        messages.extend(kept);

        let estimated_tokens_after = messages.iter().map(estimate_message_tokens).sum();
        ContextBuildResult {
            messages,
            compacted: true,
            estimated_tokens_before,
            estimated_tokens_after,
            working_memory: summary,
        }
    }
}

/// 估算文本 Token 数。
fn estimate_tokens(content: &str) -> usize {
    content.chars().count() / 2 + 1
}

/// 估算消息 Token 数。
fn estimate_message_tokens(message: &Message) -> usize {
    estimate_tokens(&message.content)
}

/// 截断较长文本用于摘要。
fn truncate(content: &str, limit: usize) -> String {
    let truncated: String = content.chars().take(limit).collect();
    if content.chars().count() > limit {
        format!("{}...", truncated)
    } else {
        truncated
    }
}
