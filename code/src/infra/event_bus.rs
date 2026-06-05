//! 事件总线实现。
//!
//! 本模块负责按 `session_id` 路由事件，并把事件审计写入 `event_logs`。

use crate::infra::db::SqliteDatabase;
use crate::infra::repository::{
    AgentRepository, EventLogInsertInput, EventLogRepository, RepositoryError, SessionRepository,
};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::rc::Rc;

/// 事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    UserMessageReceived,
    ModelThinkingStarted,
    AssistantOutputDelta,
    AssistantOutputCompleted,
    ToolStarted,
    ToolFinished,
    MetricsUpdated,
    ChildAgentCreated,
    ChildAgentResultReady,
    ChildAgentDestroyed,
    ContextCompressed,
    ErrorRaised,
}

impl EventType {
    /// 返回事件类型字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::UserMessageReceived => "user_message_received",
            EventType::ModelThinkingStarted => "model_thinking_started",
            EventType::AssistantOutputDelta => "assistant_output_delta",
            EventType::AssistantOutputCompleted => "assistant_output_completed",
            EventType::ToolStarted => "tool_started",
            EventType::ToolFinished => "tool_finished",
            EventType::MetricsUpdated => "metrics_updated",
            EventType::ChildAgentCreated => "child_agent_created",
            EventType::ChildAgentResultReady => "child_agent_result_ready",
            EventType::ChildAgentDestroyed => "child_agent_destroyed",
            EventType::ContextCompressed => "context_compressed",
            EventType::ErrorRaised => "error_raised",
        }
    }
}

/// 事件载荷。
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnvelope {
    /// 事件标识。
    pub event_id: String,
    /// 轮次标识。
    pub round_id: Option<String>,
    /// 事件类型。
    pub event_type: String,
    /// 目标会话标识。
    pub session_id: String,
    /// 事件生产者智能体标识。
    pub agent_id: Option<String>,
    /// 目标智能体标识。
    pub target_agent_id: Option<String>,
    /// 来源会话标识。
    pub source_session_id: Option<String>,
    /// 事件时间。
    pub timestamp: String,
    /// 原始载荷。
    pub payload: Value,
    /// 当前投递状态。
    pub status: String,
}

impl EventEnvelope {
    /// 构造基础事件。
    pub fn new(
        event_type: EventType,
        session_id: &str,
        agent_id: Option<&str>,
        round_id: Option<&str>,
        payload: Value,
    ) -> Self {
        Self {
            event_id: String::new(),
            round_id: round_id.map(ToString::to_string),
            event_type: event_type.as_str().to_string(),
            session_id: session_id.to_string(),
            agent_id: agent_id.map(ToString::to_string),
            target_agent_id: None,
            source_session_id: None,
            timestamp: SessionRepository::current_timestamp(),
            payload,
            status: "queued".to_string(),
        }
    }

    /// 附加来源会话与目标智能体。
    pub fn with_routing(
        mut self,
        source_session_id: Option<&str>,
        target_agent_id: Option<&str>,
    ) -> Self {
        self.source_session_id = source_session_id.map(ToString::to_string);
        self.target_agent_id = target_agent_id.map(ToString::to_string);
        self
    }
}

/// 事件总线错误。
#[derive(Debug)]
pub enum EventBusError {
    RepositoryFailed(String),
}

impl Display for EventBusError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EventBusError::RepositoryFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for EventBusError {}

impl From<RepositoryError> for EventBusError {
    fn from(value: RepositoryError) -> Self {
        EventBusError::RepositoryFailed(value.to_string())
    }
}

/// 事件总线。
#[derive(Clone)]
pub struct EventBus<'a> {
    database: &'a SqliteDatabase,
    queues_by_session: Rc<RefCell<HashMap<String, VecDeque<EventEnvelope>>>>,
}

impl<'a> EventBus<'a> {
    /// 构造事件总线。
    pub fn new(database: &'a SqliteDatabase) -> Self {
        Self {
            database,
            queues_by_session: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// 注册会话订阅队列。
    pub fn register_session(&self, session_id: &str) -> Result<(), EventBusError> {
        let mut queues = self.queues_by_session.borrow_mut();
        queues
            .entry(session_id.to_string())
            .or_insert_with(VecDeque::new);
        Ok(())
    }

    /// 投递事件。
    pub fn publish(&self, mut event: EventEnvelope) -> Result<EventEnvelope, EventBusError> {
        let mut queues = self.queues_by_session.borrow_mut();
        if let Some(queue) = queues.get_mut(&event.session_id) {
            event.status = "handled".to_string();
            queue.push_back(event.clone());
        } else {
            event.status = "dropped".to_string();
        }

        if self.session_exists(&event.session_id)? {
            let repository = EventLogRepository::new(self.database.connection());
            event.event_id = repository.next_event_id()?;
            repository.insert(EventLogInsertInput {
                event_id: event.event_id.clone(),
                round_id: event.round_id.clone(),
                source_session_id: event.source_session_id.clone(),
                session_id: event.session_id.clone(),
                agent_id: self.normalize_agent_id(event.agent_id.as_deref())?,
                target_agent_id: self.normalize_agent_id(event.target_agent_id.as_deref())?,
                event_type: event.event_type.clone(),
                payload_summary: summarize_payload(&event.payload),
                status: event.status.clone(),
            })?;
        }

        Ok(event)
    }

    /// 拉取指定会话的全部待消费事件。
    pub fn drain_session_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<EventEnvelope>, EventBusError> {
        let mut queues = self.queues_by_session.borrow_mut();
        let Some(queue) = queues.get_mut(session_id) else {
            return Ok(Vec::new());
        };

        Ok(queue.drain(..).collect())
    }

    fn session_exists(&self, session_id: &str) -> Result<bool, EventBusError> {
        Ok(SessionRepository::new(self.database.connection())
            .get_by_id(session_id)?
            .is_some())
    }

    fn normalize_agent_id(&self, agent_id: Option<&str>) -> Result<Option<String>, EventBusError> {
        let Some(agent_id) = agent_id else {
            return Ok(None);
        };

        Ok(
            if AgentRepository::new(self.database.connection())
                .get_by_id(agent_id)?
                .is_some()
            {
                Some(agent_id.to_string())
            } else {
                None
            },
        )
    }
}

fn summarize_payload(payload: &Value) -> String {
    match serde_json::to_string(payload) {
        Ok(text) => {
            if text.len() > 200 {
                text.chars().take(200).collect()
            } else {
                text
            }
        }
        Err(_) => "事件载荷摘要序列化失败。".to_string(),
    }
}
