use dshns_core::message::Message;
use dshns_core::config::ContextConfig;

pub struct ContextManager { config: ContextConfig }

impl ContextManager {
    pub fn new(config: ContextConfig) -> Self { Self { config } }

    pub fn build_messages(&self, system_prompt: &str, history: &[Message], input: &str) -> Vec<Message> {
        let mut msgs = vec![Message::System { content: system_prompt.into() }];
        msgs.extend(history.iter().cloned());
        msgs.push(Message::User { content: input.into() });
        let estimated = self.estimate(&msgs);
        let threshold = (self.config.max_window_tokens as f64 * self.config.compression_threshold) as usize;
        if estimated > threshold { msgs = self.compress(msgs); }
        msgs
    }

    fn estimate(&self, msgs: &[Message]) -> usize {
        msgs.iter().map(|m| match m {
            Message::System { content } | Message::User { content } => content.len(),
            Message::Assistant { content, tool_calls } => {
                content.as_ref().map(|c| c.len()).unwrap_or(0)
                + tool_calls.as_ref().map(|v| v.iter().map(|t| t.function.arguments.len()).sum::<usize>()).unwrap_or(0)
            }
            Message::Tool { content, .. } => content.len(),
        }).sum::<usize>() / 3
    }

    fn compress(&self, msgs: Vec<Message>) -> Vec<Message> {
        let system = msgs.first().cloned();
        let mut result = vec![system.unwrap()];
        let total = msgs.len();
        for (i, msg) in msgs.into_iter().skip(1).enumerate() {
            match &msg {
                Message::Tool { content, .. } if i < total.saturating_sub(7) => {
                    result.push(Message::Tool { tool_call_id: "truncated".into(), content: self.truncate(content, self.config.max_tool_result_tokens) });
                }
                _ => result.push(msg),
            }
        }
        result
    }

    pub fn truncate(&self, content: &str, max_tokens: usize) -> String {
        let max_chars = max_tokens * 3;
        if content.len() <= max_chars { return content.into(); }
        let head: String = content.chars().take(max_chars * 4 / 10).collect();
        let tail: String = content.chars().rev().take(max_chars * 5 / 10).collect::<String>().chars().rev().collect();
        format!("{}\n\n[... {} 字符已省略 ...]\n\n{}", head, content.len() - head.len() - tail.len(), tail)
    }
}
