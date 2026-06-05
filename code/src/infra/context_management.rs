//! 上下文管理器实现。
//!
//! 本模块负责 `Token` 估算、压缩触发判断、压缩记录落盘与长结果预算策略。

use crate::domain::workspace_session::MessageRecord;
use crate::infra::db::SqliteDatabase;
use crate::infra::repository::{
    AgentRepository, ContextCompressionRepository, MessageRepository, RepositoryError,
};

/// 压缩触发原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionReason {
    /// 超过最大上下文。
    OverLimit,
    /// 接近最大上下文阈值。
    NearLimit,
}

impl CompressionReason {
    fn as_storage_value(&self) -> &'static str {
        match self {
            CompressionReason::OverLimit => "over_limit",
            CompressionReason::NearLimit => "near_limit",
        }
    }
}

/// 长结果处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongResultStrategy {
    /// 直接返回。
    Direct,
    /// 先压缩上下文再拼接短结果。
    DirectAfterCompression,
    /// 生成摘要。
    SummaryGenerated,
    /// 截断最后 500 个字符。
    TruncateLast500Chars,
}

/// 上下文管理配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextManagerConfig {
    /// 保留的最新可见消息数量。
    pub keep_message_count: usize,
    /// 普通模型安全余量。
    pub token_safety_margin: u32,
    /// `[1m]` 模型安全余量。
    pub token_safety_margin_1m: u32,
    /// 长结果直接摘要阈值。
    pub summary_threshold_chars: usize,
}

impl Default for ContextManagerConfig {
    fn default() -> Self {
        Self {
            keep_message_count: 4,
            token_safety_margin: 8_192,
            token_safety_margin_1m: 32_768,
            summary_threshold_chars: 500,
        }
    }
}

/// 压缩结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompressionSummary {
    /// 压缩摘要文本。
    pub summary_text: String,
    /// 保留的最新消息。
    pub kept_messages: Vec<MessageRecord>,
    /// 触发原因。
    pub trigger_reason: CompressionReason,
}

/// 长结果预算输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongResultBudgetInput {
    /// 原始内容。
    pub content: String,
    /// 是否为失败结果。
    pub is_failure: bool,
    /// 当前上下文 `Token` 估算。
    pub context_tokens_before_result: u32,
    /// 最大上下文。
    pub max_context: u32,
    /// 工具元信息估算。
    pub tool_tokens: u32,
    /// 技能元信息估算。
    pub skill_tokens: u32,
    /// 预计输出预算。
    pub expected_output_tokens: u32,
    /// 当前模型名。
    pub model_name: String,
    /// 摘要子智能体是否可用。
    pub summary_agent_available: bool,
}

/// 长结果预算输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongResultBudgetResult {
    /// 处理后的内容。
    pub content: String,
    /// 采用的策略。
    pub strategy: LongResultStrategy,
}

/// 上下文管理器。
pub struct ContextManager<'a> {
    database: &'a SqliteDatabase,
    config: ContextManagerConfig,
}

impl<'a> ContextManager<'a> {
    /// 构造上下文管理器。
    pub fn new(database: &'a SqliteDatabase, config: ContextManagerConfig) -> Self {
        Self { database, config }
    }

    /// 估算文本的 `Token` 数。
    pub fn estimate_tokens(&self, text: &str) -> u32 {
        let char_count = text.chars().count() as u32;
        (char_count / 4).max(1)
    }

    /// 获取模型对应的安全余量。
    pub fn token_safety_margin_for_model(&self, model_name: &str) -> u32 {
        if model_name.contains("[1m]") {
            self.config.token_safety_margin_1m
        } else {
            self.config.token_safety_margin
        }
    }

    /// 判断是否需要压缩。
    pub fn evaluate_compression_need(
        &self,
        estimated_tokens: u32,
        tool_tokens: u32,
        skill_tokens: u32,
        max_context: u32,
        expected_output_tokens: u32,
        model_name: &str,
    ) -> Option<CompressionReason> {
        let safety_margin = self.token_safety_margin_for_model(model_name);
        if estimated_tokens + tool_tokens + skill_tokens + safety_margin + expected_output_tokens
            > max_context
        {
            return Some(CompressionReason::OverLimit);
        }

        if estimated_tokens + tool_tokens + skill_tokens + safety_margin
            > ((max_context as f64) * 0.85) as u32
        {
            return Some(CompressionReason::NearLimit);
        }

        None
    }

    /// 压缩指定会话的旧消息，并写入压缩记录。
    pub fn compress_session_context(
        &self,
        session_id: &str,
        agent_id: &str,
        estimated_tokens_before: u32,
        trigger_reason: CompressionReason,
    ) -> Result<ContextCompressionSummary, RepositoryError> {
        let message_repository = MessageRepository::new(self.database.connection());
        let agent_repository = AgentRepository::new(self.database.connection());
        if agent_repository.get_by_id(agent_id)?.is_none() {
            let _ = agent_repository.create_primary_agent(session_id, agent_id)?;
        }
        let all_messages = message_repository.list_by_session_id(session_id)?;
        let visible_messages = all_messages
            .into_iter()
            .filter(|message| {
                message.include_in_context
                    && message.content_type != "command_audit"
                    && !message.is_compressed_source
            })
            .collect::<Vec<_>>();

        if visible_messages.len() <= self.config.keep_message_count {
            return Err(RepositoryError::QueryFailed(
                "压缩后仍超上下文上限，当前没有足够的旧消息可压缩。".to_string(),
            ));
        }

        let split_index = visible_messages.len() - self.config.keep_message_count;
        let compressible_messages = visible_messages[..split_index].to_vec();
        let kept_messages = visible_messages[split_index..].to_vec();
        let summary_text = self.generate_summary_text(&compressible_messages);
        let estimated_tokens_after = self.estimate_tokens(&summary_text)
            + kept_messages
                .iter()
                .map(|message| self.estimate_tokens(&message.content))
                .sum::<u32>();

        message_repository.mark_messages_as_compressed_source(
            &compressible_messages
                .iter()
                .map(|message| message.message_id.clone())
                .collect::<Vec<_>>(),
        )?;

        let compression_repository = ContextCompressionRepository::new(self.database.connection());
        let _record = compression_repository.create(
            session_id,
            agent_id,
            &compressible_messages
                .first()
                .expect("至少存在一条可压缩消息")
                .message_id,
            &compressible_messages
                .last()
                .expect("至少存在一条可压缩消息")
                .message_id,
            &summary_text,
            kept_messages.len() as i64,
            trigger_reason.as_storage_value(),
            estimated_tokens_before as i64,
            estimated_tokens_after as i64,
        )?;

        Ok(ContextCompressionSummary {
            summary_text,
            kept_messages,
            trigger_reason,
        })
    }

    /// 处理长结果预算。
    pub fn handle_long_result(&self, input: LongResultBudgetInput) -> LongResultBudgetResult {
        let content_tokens = self.estimate_tokens(&input.content);
        let safety_margin = self.token_safety_margin_for_model(&input.model_name);
        let total_after_append = input.context_tokens_before_result
            + input.tool_tokens
            + input.skill_tokens
            + safety_margin
            + input.expected_output_tokens
            + content_tokens;

        if total_after_append <= input.max_context {
            return LongResultBudgetResult {
                content: input.content,
                strategy: LongResultStrategy::Direct,
            };
        }

        let char_count = input.content.chars().count();
        if char_count <= self.config.summary_threshold_chars {
            return LongResultBudgetResult {
                content: input.content,
                strategy: LongResultStrategy::DirectAfterCompression,
            };
        }

        if input.summary_agent_available && char_count <= self.config.summary_threshold_chars * 8 {
            return LongResultBudgetResult {
                content: self.generate_long_result_summary(&input.content),
                strategy: LongResultStrategy::SummaryGenerated,
            };
        }

        LongResultBudgetResult {
            content: truncate_last_n_chars(&input.content, 500),
            strategy: LongResultStrategy::TruncateLast500Chars,
        }
    }

    fn generate_summary_text(&self, messages: &[MessageRecord]) -> String {
        let mut lines = vec!["压缩摘要：以下为较早历史消息的关键信息。".to_string()];
        for message in messages {
            let snippet = truncate_first_n_chars(&message.content.replace('\n', " "), 120);
            lines.push(format!("[{}] {}", message.role, snippet));
        }
        lines.join("\n")
    }

    fn generate_long_result_summary(&self, content: &str) -> String {
        let head = truncate_first_n_chars(content, 180);
        let tail = truncate_last_n_chars(content, 180);
        format!("摘要子智能体结果摘要：前文={head}；结尾={tail}")
    }
}

fn truncate_first_n_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn truncate_last_n_chars(text: &str, limit: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(limit);
    chars[start..].iter().collect()
}
