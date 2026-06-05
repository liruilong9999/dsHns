//! 子智能体管理器实现。
//!
//! 本模块负责创建、派发、销毁子智能体，并维护父子关系与生命周期。

use crate::domain::runtime::{AgentRecord, AgentRelationRecord};
use crate::infra::event_bus::{EventBus, EventEnvelope, EventType};
use crate::infra::repository::{
    AgentRelationRepository, AgentRepository, RepositoryError, SessionRepository,
};
use serde_json::json;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// 子智能体模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildAgentMode {
    /// 继承模式。
    Inherit,
    /// 隔离模式。
    Isolated,
}

impl ChildAgentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChildAgentMode::Inherit => "inherit",
            ChildAgentMode::Isolated => "isolated",
        }
    }
}

/// 子智能体管理配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildAgentManagerConfig {
    /// 最大子智能体层级。
    pub max_child_depth: i64,
    /// 全局最大子智能体总数。
    pub max_child_agents_total: i64,
}

impl Default for ChildAgentManagerConfig {
    fn default() -> Self {
        Self {
            max_child_depth: 1,
            max_child_agents_total: 5,
        }
    }
}

/// 创建子智能体请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateChildAgentRequest {
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub mode: ChildAgentMode,
    pub task_summary: String,
    pub inherited_context: Option<String>,
}

/// 继续派发子智能体请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildAgentDispatchRequest {
    pub child_agent_id: String,
    pub task_summary: String,
    pub result_summary: Option<String>,
}

/// 创建子智能体结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildAgentCreatedResult {
    pub child_agent_id: String,
    pub child_session_id: String,
    pub mode: ChildAgentMode,
}

/// 派发/销毁结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildAgentLifecycleResult {
    pub child_agent_id: String,
    pub child_session_id: String,
    pub current_status: String,
}

/// 子智能体管理错误。
#[derive(Debug)]
pub enum ChildAgentManagerError {
    RepositoryFailed(String),
    ValidationFailed(String),
}

impl Display for ChildAgentManagerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ChildAgentManagerError::RepositoryFailed(message) => write!(f, "{message}"),
            ChildAgentManagerError::ValidationFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for ChildAgentManagerError {}

impl From<RepositoryError> for ChildAgentManagerError {
    fn from(value: RepositoryError) -> Self {
        ChildAgentManagerError::RepositoryFailed(value.to_string())
    }
}

/// 子智能体管理器。
#[derive(Clone)]
pub struct ChildAgentManager<'a> {
    database: &'a crate::infra::db::SqliteDatabase,
    event_bus: EventBus<'a>,
    config: ChildAgentManagerConfig,
}

impl<'a> ChildAgentManager<'a> {
    /// 构造子智能体管理器。
    pub fn new(
        database: &'a crate::infra::db::SqliteDatabase,
        event_bus: EventBus<'a>,
        config: ChildAgentManagerConfig,
    ) -> Self {
        Self {
            database,
            event_bus,
            config,
        }
    }

    /// 创建子智能体。
    pub fn create_child_agent(
        &self,
        request: CreateChildAgentRequest,
    ) -> Result<ChildAgentCreatedResult, ChildAgentManagerError> {
        let session_repository = SessionRepository::new(self.database.connection());
        let parent_session = session_repository
            .get_by_id(&request.parent_session_id)?
            .ok_or_else(|| {
                ChildAgentManagerError::ValidationFailed(format!(
                    "父会话不存在：{}",
                    request.parent_session_id
                ))
            })?;

        let agent_repository = AgentRepository::new(self.database.connection());
        let parent_agent = self.ensure_parent_agent(
            &agent_repository,
            &request.parent_session_id,
            &request.parent_agent_id,
        )?;

        if parent_agent.depth + 1 > self.config.max_child_depth {
            return Err(ChildAgentManagerError::ValidationFailed(format!(
                "子智能体层级超限，当前最大深度为 {}。",
                self.config.max_child_depth
            )));
        }

        if agent_repository.count_active_children()? >= self.config.max_child_agents_total {
            return Err(ChildAgentManagerError::ValidationFailed(format!(
                "子智能体数量已达到上限 {}。",
                self.config.max_child_agents_total
            )));
        }

        let child_session_id = if request.mode == ChildAgentMode::Isolated {
            let created_session = session_repository.create(
                &parent_session.workspace_id,
                &request.task_summary,
                &parent_session.current_model,
                &parent_session.session_approval_mode,
                parent_session.context_limit,
                None,
                &SessionRepository::current_timestamp(),
            )?;
            created_session.session_id
        } else {
            request.parent_session_id.clone()
        };

        let child_record = agent_repository.create_child_agent(
            &child_session_id,
            &request.parent_agent_id,
            request.mode.as_str(),
            parent_agent.depth + 1,
            "created",
            &request.task_summary,
        )?;

        let relation_repository = AgentRelationRepository::new(self.database.connection());
        relation_repository.create(
            &request.parent_agent_id,
            &child_record.agent_id,
            request.mode.as_str(),
            Some(request.task_summary.clone()),
            request.inherited_context,
        )?;

        let _ = self.event_bus.publish(EventEnvelope::new(
            EventType::ChildAgentCreated,
            &request.parent_session_id,
            Some(&request.parent_agent_id),
            None,
            json!({
                "child_agent_id": child_record.agent_id,
                "child_session_id": child_session_id,
                "mode": request.mode.as_str()
            }),
        ));

        Ok(ChildAgentCreatedResult {
            child_agent_id: child_record.agent_id,
            child_session_id,
            mode: request.mode,
        })
    }

    /// 继续派发子智能体。
    pub fn dispatch_child_agent(
        &self,
        request: ChildAgentDispatchRequest,
    ) -> Result<ChildAgentLifecycleResult, ChildAgentManagerError> {
        let agent_repository = AgentRepository::new(self.database.connection());
        let child = agent_repository
            .get_by_id(&request.child_agent_id)?
            .ok_or_else(|| {
                ChildAgentManagerError::ValidationFailed(format!(
                    "子智能体不存在：{}",
                    request.child_agent_id
                ))
            })?;
        if child.status == "destroyed" {
            return Err(ChildAgentManagerError::ValidationFailed(format!(
                "子智能体已销毁，不能继续派发：{}",
                child.agent_id
            )));
        }

        agent_repository.update_status_and_task(
            &child.agent_id,
            "running",
            &request.task_summary,
        )?;
        agent_repository.update_status_and_task(
            &child.agent_id,
            "waiting",
            &request.task_summary,
        )?;

        let relation_repository = AgentRelationRepository::new(self.database.connection());
        let relation = relation_repository
            .get_by_child_agent(&child.agent_id)?
            .ok_or_else(|| {
                ChildAgentManagerError::ValidationFailed(format!(
                    "子智能体关系不存在：{}",
                    child.agent_id
                ))
            })?;
        relation_repository
            .update_result_summary(&relation.relation_id, request.result_summary.clone())?;

        let _ = self.event_bus.publish(
            EventEnvelope::new(
                EventType::ChildAgentResultReady,
                &relation.parent_agent_id_session(&agent_repository)?,
                Some(&child.agent_id),
                None,
                json!({
                    "child_agent_id": child.agent_id,
                    "status": "waiting"
                }),
            )
            .with_routing(Some(&child.session_id), Some(&relation.parent_agent_id)),
        );

        Ok(ChildAgentLifecycleResult {
            child_agent_id: child.agent_id,
            child_session_id: child.session_id,
            current_status: "waiting".to_string(),
        })
    }

    /// 销毁子智能体。
    pub fn destroy_child_agent(
        &self,
        child_agent_id: &str,
    ) -> Result<ChildAgentLifecycleResult, ChildAgentManagerError> {
        let agent_repository = AgentRepository::new(self.database.connection());
        let child = agent_repository.get_by_id(child_agent_id)?.ok_or_else(|| {
            ChildAgentManagerError::ValidationFailed(format!("子智能体不存在：{}", child_agent_id))
        })?;
        agent_repository.update_status_and_task(
            &child.agent_id,
            "destroyed",
            child.task_summary.as_deref().unwrap_or(""),
        )?;

        let _ = self.event_bus.publish(EventEnvelope::new(
            EventType::ChildAgentDestroyed,
            &child.session_id,
            Some(&child.agent_id),
            None,
            json!({
                "child_agent_id": child.agent_id,
                "status": "destroyed"
            }),
        ));

        Ok(ChildAgentLifecycleResult {
            child_agent_id: child.agent_id,
            child_session_id: child.session_id,
            current_status: "destroyed".to_string(),
        })
    }

    fn ensure_parent_agent(
        &self,
        repository: &AgentRepository<'_>,
        session_id: &str,
        parent_agent_id: &str,
    ) -> Result<AgentRecord, ChildAgentManagerError> {
        if let Some(parent) = repository.get_by_id(parent_agent_id)? {
            return Ok(parent);
        }

        Ok(repository.create_primary_agent(session_id, parent_agent_id)?)
    }
}

trait ParentAgentSessionLookup {
    fn parent_agent_id_session(
        &self,
        repository: &AgentRepository<'_>,
    ) -> Result<String, ChildAgentManagerError>;
}

impl ParentAgentSessionLookup for AgentRelationRecord {
    fn parent_agent_id_session(
        &self,
        repository: &AgentRepository<'_>,
    ) -> Result<String, ChildAgentManagerError> {
        let parent = repository
            .get_by_id(&self.parent_agent_id)?
            .ok_or_else(|| {
                ChildAgentManagerError::ValidationFailed(format!(
                    "父智能体不存在：{}",
                    self.parent_agent_id
                ))
            })?;
        Ok(parent.session_id)
    }
}
