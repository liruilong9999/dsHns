//! 运行指标模块。
//!
//! 本模块负责 `session_metrics` 快照落盘与指标事件刷新。

use crate::domain::runtime::SessionMetricRecord;
use crate::infra::db::SqliteDatabase;
use crate::infra::event_bus::{EventBus, EventEnvelope, EventType};
use crate::infra::repository::{
    AgentRepository, RepositoryError, SessionRepository, WorkspaceRepository,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

/// 指标快照写入请求。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMetricInput {
    /// 会话标识。
    pub session_id: String,
    /// 智能体标识。
    pub agent_id: Option<String>,
    /// 输入 Token。
    pub input_tokens: i64,
    /// 输出 Token。
    pub output_tokens: i64,
    /// 缓存命中率。
    pub cache_hit_rate: f64,
    /// 剩余上下文。
    pub remaining_context: i64,
    /// 工具成功次数。
    pub tool_success_count: i64,
    /// 工具失败次数。
    pub tool_failure_count: i64,
    /// 当前活跃工具数。
    pub active_tool_calls: i64,
}

/// 指标仓储。
pub struct SessionMetricsRepository<'a> {
    connection: &'a Connection,
}

impl<'a> SessionMetricsRepository<'a> {
    /// 构造指标仓储。
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// 写入指标快照。
    pub fn insert(
        &self,
        input: SessionMetricInput,
    ) -> Result<SessionMetricRecord, RepositoryError> {
        let metric_id = WorkspaceRepository::next_identifier(
            self.connection,
            "session_metrics",
            "metric_id",
            "MET",
        )?;
        let active_sessions = self.count_active_sessions()?;
        let active_child_agents = self.count_active_child_agents()?;
        let now = SessionRepository::current_timestamp();
        let agent_id = self.normalize_agent_id(input.agent_id.as_deref())?;

        self.connection
            .execute(
                "INSERT INTO session_metrics
                 (metric_id, session_id, agent_id, input_tokens, output_tokens, cache_hit_rate, remaining_context, tool_success_count, tool_failure_count, active_sessions, active_child_agents, active_tool_calls, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    metric_id,
                    input.session_id,
                    agent_id,
                    input.input_tokens,
                    input.output_tokens,
                    input.cache_hit_rate,
                    input.remaining_context,
                    input.tool_success_count,
                    input.tool_failure_count,
                    active_sessions,
                    active_child_agents,
                    input.active_tool_calls,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!("写入会话指标失败，原因：{error}"))
            })?;

        self.get_by_id(&metric_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("指标快照未找到：{metric_id}")))
    }

    /// 查询指定会话的最新指标。
    pub fn latest_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionMetricRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT metric_id, session_id, agent_id, input_tokens, output_tokens, cache_hit_rate, remaining_context, tool_success_count, tool_failure_count, active_sessions, active_child_agents, active_tool_calls, created_at
                 FROM session_metrics
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![session_id],
                Self::map_metric_record,
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询最新会话指标失败，session_id：{session_id}，原因：{error}"
                ))
            })
    }

    fn get_by_id(&self, metric_id: &str) -> Result<Option<SessionMetricRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT metric_id, session_id, agent_id, input_tokens, output_tokens, cache_hit_rate, remaining_context, tool_success_count, tool_failure_count, active_sessions, active_child_agents, active_tool_calls, created_at
                 FROM session_metrics WHERE metric_id = ?1 LIMIT 1",
                params![metric_id],
                Self::map_metric_record,
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询指标快照失败，metric_id：{metric_id}，原因：{error}"
                ))
            })
    }

    fn map_metric_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMetricRecord> {
        Ok(SessionMetricRecord {
            metric_id: row.get(0)?,
            session_id: row.get(1)?,
            agent_id: row.get(2)?,
            input_tokens: row.get(3)?,
            output_tokens: row.get(4)?,
            cache_hit_rate: row.get(5)?,
            remaining_context: row.get(6)?,
            tool_success_count: row.get(7)?,
            tool_failure_count: row.get(8)?,
            active_sessions: row.get(9)?,
            active_child_agents: row.get(10)?,
            active_tool_calls: row.get(11)?,
            created_at: row.get(12)?,
        })
    }

    fn count_active_sessions(&self) -> Result<i64, RepositoryError> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!("统计活跃会话失败，原因：{error}"))
            })
    }

    fn count_active_child_agents(&self) -> Result<i64, RepositoryError> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE agent_mode != 'primary' AND status != 'destroyed'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!("统计活跃子智能体失败，原因：{error}"))
            })
    }

    fn normalize_agent_id(
        &self,
        agent_id: Option<&str>,
    ) -> Result<Option<String>, RepositoryError> {
        let Some(agent_id) = agent_id else {
            return Ok(None);
        };

        Ok(
            if AgentRepository::new(self.connection)
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

/// 指标采集器。
pub struct MetricsCollector<'a> {
    database: &'a SqliteDatabase,
    event_bus: Option<EventBus<'a>>,
}

impl<'a> MetricsCollector<'a> {
    /// 构造指标采集器。
    pub fn new(database: &'a SqliteDatabase) -> Self {
        Self {
            database,
            event_bus: None,
        }
    }

    /// 注入事件总线。
    pub fn with_event_bus(mut self, event_bus: EventBus<'a>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// 记录一轮会话指标，并在可用时发出 `metrics_updated` 事件。
    pub fn record_snapshot(
        &self,
        input: SessionMetricInput,
    ) -> Result<SessionMetricRecord, RepositoryError> {
        let repository = SessionMetricsRepository::new(self.database.connection());
        let snapshot = repository.insert(input.clone())?;

        if let Some(event_bus) = &self.event_bus {
            let _ = event_bus.publish(EventEnvelope::new(
                EventType::MetricsUpdated,
                &snapshot.session_id,
                snapshot.agent_id.as_deref(),
                None,
                json!({
                    "input_tokens": snapshot.input_tokens,
                    "output_tokens": snapshot.output_tokens,
                    "cache_hit_rate": snapshot.cache_hit_rate,
                    "remaining_context": snapshot.remaining_context,
                    "tool_success_count": snapshot.tool_success_count,
                    "tool_failure_count": snapshot.tool_failure_count,
                    "active_sessions": snapshot.active_sessions,
                    "active_child_agents": snapshot.active_child_agents,
                    "active_tool_calls": snapshot.active_tool_calls
                }),
            ));
        }

        Ok(snapshot)
    }
}
