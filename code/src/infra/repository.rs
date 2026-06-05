//! 仓储模块。
//!
//! 本模块负责 `workspaces`、`sessions`、`messages` 三类核心实体的
//! `SQLite` 持久化访问，实现 `TASK-005` 的基础仓储闭环。

use crate::domain::runtime::{
    AgentRecord, AgentRelationRecord, ContextCompressionRecord, EventLogRecord,
};
use crate::domain::workspace_session::{MessageRecord, SessionRecord, WorkspaceRecord};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// 仓储错误。
#[derive(Debug)]
pub enum RepositoryError {
    /// 数据库访问失败。
    QueryFailed(String),
    /// 请求对象不存在。
    NotFound(String),
}

impl Display for RepositoryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RepositoryError::QueryFailed(message) => write!(f, "{message}"),
            RepositoryError::NotFound(message) => write!(f, "{message}"),
        }
    }
}

impl Error for RepositoryError {}

/// 目录仓储。
pub struct WorkspaceRepository<'a> {
    /// 数据库连接。
    connection: &'a Connection,
}

impl<'a> WorkspaceRepository<'a> {
    /// 构造目录仓储。
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// 创建目录记录。
    pub fn create(&self, name: &str, root_path: &str) -> Result<WorkspaceRecord, RepositoryError> {
        let workspace_id =
            Self::next_identifier(self.connection, "workspaces", "workspace_id", "WS")?;
        let now = SessionRepository::current_timestamp();

        self.connection
            .execute(
                "INSERT INTO workspaces (workspace_id, name, root_path, created_at, updated_at, is_deleted)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                params![workspace_id, name, root_path, now, now],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建目录记录失败，root_path：{root_path}，原因：{error}"
                ))
            })?;

        self.get_by_id_including_deleted(&workspace_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("目录创建后未找到：{workspace_id}")))
    }

    /// 通过路径查询目录，包含逻辑删除数据。
    pub fn get_by_root_path_including_deleted(
        &self,
        root_path: &str,
    ) -> Result<Option<WorkspaceRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT workspace_id, name, root_path, created_at, updated_at, is_deleted
                 FROM workspaces
                 WHERE root_path = ?1
                 LIMIT 1",
                params![root_path],
                |row| {
                    Ok(WorkspaceRecord {
                        workspace_id: row.get(0)?,
                        name: row.get(1)?,
                        root_path: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        is_deleted: row.get::<_, bool>(5)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "按路径查询目录失败，root_path：{root_path}，原因：{error}"
                ))
            })
    }

    /// 按标识查询目录，包含逻辑删除数据。
    pub fn get_by_id_including_deleted(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT workspace_id, name, root_path, created_at, updated_at, is_deleted
                 FROM workspaces
                 WHERE workspace_id = ?1
                 LIMIT 1",
                params![workspace_id],
                |row| {
                    Ok(WorkspaceRecord {
                        workspace_id: row.get(0)?,
                        name: row.get(1)?,
                        root_path: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        is_deleted: row.get::<_, bool>(5)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "按标识查询目录失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })
    }

    /// 更新目录名称。
    pub fn update_name(
        &self,
        workspace_id: &str,
        name: &str,
    ) -> Result<WorkspaceRecord, RepositoryError> {
        let now = SessionRepository::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE workspaces
                 SET name = ?1, updated_at = ?2
                 WHERE workspace_id = ?3",
                params![name, now, workspace_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新目录名称失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })?;

        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "目录不存在，无法更新：{workspace_id}"
            )));
        }

        self.get_by_id_including_deleted(workspace_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("目录不存在：{workspace_id}")))
    }

    /// 恢复已被逻辑删除的目录。
    pub fn restore(
        &self,
        workspace_id: &str,
        name: &str,
    ) -> Result<WorkspaceRecord, RepositoryError> {
        let now = SessionRepository::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE workspaces
                 SET name = ?1, updated_at = ?2, is_deleted = 0
                 WHERE workspace_id = ?3",
                params![name, now, workspace_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "恢复目录失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })?;

        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "目录不存在，无法恢复：{workspace_id}"
            )));
        }

        self.get_by_id_including_deleted(workspace_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("目录不存在：{workspace_id}")))
    }

    /// 逻辑删除目录。
    pub fn mark_deleted(&self, workspace_id: &str) -> Result<(), RepositoryError> {
        let now = SessionRepository::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE workspaces
                 SET is_deleted = 1, updated_at = ?1
                 WHERE workspace_id = ?2",
                params![now, workspace_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "逻辑删除目录失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })?;

        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "目录不存在，无法删除：{workspace_id}"
            )));
        }

        Ok(())
    }

    /// 生成下一个目录标识。
    pub(crate) fn next_identifier(
        connection: &Connection,
        table_name: &str,
        id_column_name: &str,
        prefix: &str,
    ) -> Result<String, RepositoryError> {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
                row.get(0)
            })
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "生成标识失败，表：{table_name}，原因：{error}"
                ))
            })?;

        let _ = id_column_name;
        Ok(format!("{prefix}-{next:04}", next = count + 1))
    }
}

/// 会话仓储。
pub struct SessionRepository<'a> {
    /// 数据库连接。
    connection: &'a Connection,
}

impl<'a> SessionRepository<'a> {
    /// 构造会话仓储。
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// 创建会话记录。
    pub fn create(
        &self,
        workspace_id: &str,
        title: &str,
        current_model: &str,
        session_approval_mode: &str,
        context_limit: i64,
        last_message_at: Option<String>,
        now: &str,
    ) -> Result<SessionRecord, RepositoryError> {
        let session_id =
            WorkspaceRepository::next_identifier(self.connection, "sessions", "session_id", "SES")?;

        self.connection
            .execute(
                "INSERT INTO sessions
                 (session_id, workspace_id, title, status, current_model, session_approval_mode, context_limit, last_message_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'active', ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    session_id,
                    workspace_id,
                    title,
                    current_model,
                    session_approval_mode,
                    context_limit,
                    last_message_at,
                    now,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建会话失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })?;

        self.get_by_id(&session_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("会话创建后未找到：{session_id}")))
    }

    /// 按标识查询会话。
    pub fn get_by_id(&self, session_id: &str) -> Result<Option<SessionRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT session_id, workspace_id, title, status, current_model, session_approval_mode, context_limit, last_message_at, created_at, updated_at
                 FROM sessions
                 WHERE session_id = ?1
                 LIMIT 1",
                params![session_id],
                Self::map_session_record,
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "按标识查询会话失败，session_id：{session_id}，原因：{error}"
                ))
            })
    }

    /// 更新会话标题。
    pub fn update_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<SessionRecord, RepositoryError> {
        let now = Self::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE sessions
                 SET title = ?1, updated_at = ?2
                 WHERE session_id = ?3",
                params![title, now, session_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新会话标题失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "会话不存在，无法更新：{session_id}"
            )));
        }

        self.get_by_id(session_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("会话不存在：{session_id}")))
    }

    /// 更新会话当前模型与上下文上限。
    pub fn update_model(
        &self,
        session_id: &str,
        current_model: &str,
        context_limit: i64,
    ) -> Result<SessionRecord, RepositoryError> {
        let now = Self::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE sessions
                 SET current_model = ?1, context_limit = ?2, updated_at = ?3
                 WHERE session_id = ?4",
                params![current_model, context_limit, now, session_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新会话模型失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "会话不存在，无法切换模型：{session_id}"
            )));
        }

        self.get_by_id(session_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("会话不存在：{session_id}")))
    }

    /// 更新会话审批模式。
    pub fn update_approval_mode(
        &self,
        session_id: &str,
        session_approval_mode: &str,
    ) -> Result<SessionRecord, RepositoryError> {
        let now = Self::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE sessions
                 SET session_approval_mode = ?1, updated_at = ?2
                 WHERE session_id = ?3",
                params![session_approval_mode, now, session_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新会话审批模式失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "会话不存在，无法切换审批模式：{session_id}"
            )));
        }

        self.get_by_id(session_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("会话不存在：{session_id}")))
    }

    /// 按目录查询会话列表。
    pub fn list_by_workspace_id(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<SessionRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT session_id, workspace_id, title, status, current_model, session_approval_mode, context_limit, last_message_at, created_at, updated_at
                 FROM sessions
                 WHERE workspace_id = ?1
                 ORDER BY last_message_at DESC, created_at DESC",
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "准备目录会话查询失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })?;

        let rows = statement
            .query_map(params![workspace_id], Self::map_session_record)
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询目录会话失败，workspace_id：{workspace_id}，原因：{error}"
                ))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            RepositoryError::QueryFailed(format!(
                "读取目录会话结果失败，workspace_id：{workspace_id}，原因：{error}"
            ))
        })
    }

    /// 生成下一个轮次标识。
    pub fn next_round_id(&self) -> Result<String, RepositoryError> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .map_err(|error| {
                RepositoryError::QueryFailed(format!("生成轮次标识失败，原因：{error}"))
            })?;

        Ok(format!("ROUND-{next:04}", next = count + 1))
    }

    /// 获取当前 UTC 时间戳字符串。
    pub fn current_timestamp() -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    /// 把查询结果映射为会话记录。
    fn map_session_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
        Ok(SessionRecord {
            session_id: row.get(0)?,
            workspace_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            current_model: row.get(4)?,
            session_approval_mode: row.get(5)?,
            context_limit: row.get(6)?,
            last_message_at: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}

/// 消息仓储。
pub struct MessageRepository<'a> {
    /// 数据库连接。
    connection: &'a Connection,
}

impl<'a> MessageRepository<'a> {
    /// 构造消息仓储。
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// 创建首条用户消息。
    pub fn create_user_message(
        &self,
        session_id: &str,
        round_id: &str,
        content: &str,
    ) -> Result<MessageRecord, RepositoryError> {
        self.create_runtime_message(session_id, "", round_id, "user", content, "plain", true)
    }

    /// 创建命令审计消息。
    pub fn create_command_audit_message(
        &self,
        session_id: &str,
        round_id: &str,
        command_text: &str,
    ) -> Result<MessageRecord, RepositoryError> {
        self.create_runtime_message(
            session_id,
            "",
            round_id,
            "system",
            command_text,
            "command_audit",
            false,
        )
    }

    /// 创建通用运行时消息。
    pub fn create_runtime_message(
        &self,
        session_id: &str,
        agent_id: &str,
        round_id: &str,
        role: &str,
        content: &str,
        content_type: &str,
        include_in_context: bool,
    ) -> Result<MessageRecord, RepositoryError> {
        let message_id =
            WorkspaceRepository::next_identifier(self.connection, "messages", "message_id", "MSG")?;
        let sequence_no = self.next_sequence_no(session_id)?;
        let now = SessionRepository::current_timestamp();
        let agent_id_value = if agent_id.trim().is_empty() || !self.agent_exists(agent_id)? {
            None::<String>
        } else {
            Some(agent_id.to_string())
        };

        self.connection
            .execute(
                "INSERT INTO messages
                 (message_id, session_id, agent_id, round_id, sequence_no, role, content, content_type, token_estimate, include_in_context, is_compressed_source, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, 0, ?10)",
                params![
                    message_id,
                    session_id,
                    agent_id_value,
                    round_id,
                    sequence_no,
                    role,
                    content,
                    content_type,
                    include_in_context,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建运行时消息失败，session_id：{session_id}，role：{role}，原因：{error}"
                ))
            })?;

        self.connection
            .execute(
                "UPDATE sessions
                 SET last_message_at = ?1, updated_at = ?1
                 WHERE session_id = ?2",
                params![now, session_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新运行时消息时间失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        self.list_by_session_id(session_id)?
            .into_iter()
            .find(|message| message.message_id == message_id)
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("运行时消息创建后未找到：{message_id}"))
            })
    }

    /// 按会话查询消息列表。
    pub fn list_by_session_id(
        &self,
        session_id: &str,
    ) -> Result<Vec<MessageRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT message_id, session_id, agent_id, round_id, sequence_no, role, content, content_type, token_estimate, include_in_context, is_compressed_source, created_at
                 FROM messages
                 WHERE session_id = ?1
                 ORDER BY sequence_no ASC",
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "准备消息查询失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        let rows = statement
            .query_map(params![session_id], |row| {
                Ok(MessageRecord {
                    message_id: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    round_id: row.get(3)?,
                    sequence_no: row.get(4)?,
                    role: row.get(5)?,
                    content: row.get(6)?,
                    content_type: row.get(7)?,
                    token_estimate: row.get(8)?,
                    include_in_context: row.get::<_, bool>(9)?,
                    is_compressed_source: row.get::<_, bool>(10)?,
                    created_at: row.get(11)?,
                })
            })
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询消息失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            RepositoryError::QueryFailed(format!(
                "读取消息结果失败，session_id：{session_id}，原因：{error}"
            ))
        })
    }

    /// 批量把消息标记为压缩源。
    pub fn mark_messages_as_compressed_source(
        &self,
        message_ids: &[String],
    ) -> Result<(), RepositoryError> {
        for message_id in message_ids {
            self.connection
                .execute(
                    "UPDATE messages SET is_compressed_source = 1 WHERE message_id = ?1",
                    params![message_id],
                )
                .map_err(|error| {
                    RepositoryError::QueryFailed(format!(
                        "标记压缩源消息失败，message_id：{message_id}，原因：{error}"
                    ))
                })?;
        }
        Ok(())
    }

    /// 生成下一个消息顺序号。
    fn next_sequence_no(&self, session_id: &str) -> Result<i64, RepositoryError> {
        let next_sequence_no = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(sequence_no), 0) + 1 FROM messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "生成消息顺序号失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        Ok(next_sequence_no)
    }

    /// 判断智能体是否存在。
    fn agent_exists(&self, agent_id: &str) -> Result<bool, RepositoryError> {
        self.connection
            .query_row(
                "SELECT agent_id FROM agents WHERE agent_id = ?1 LIMIT 1",
                params![agent_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|result| result.is_some())
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询智能体是否存在失败，agent_id：{agent_id}，原因：{error}"
                ))
            })
    }
}

/// 上下文压缩仓储。
pub struct ContextCompressionRepository<'a> {
    connection: &'a Connection,
}

impl<'a> ContextCompressionRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn create(
        &self,
        session_id: &str,
        agent_id: &str,
        source_start_message_id: &str,
        source_end_message_id: &str,
        summary_text: &str,
        kept_message_count: i64,
        trigger_reason: &str,
        estimated_tokens_before: i64,
        estimated_tokens_after: i64,
    ) -> Result<ContextCompressionRecord, RepositoryError> {
        let compression_id = WorkspaceRepository::next_identifier(
            self.connection,
            "context_compressions",
            "compression_id",
            "CMP",
        )?;
        let now = SessionRepository::current_timestamp();
        self.connection
            .execute(
                "INSERT INTO context_compressions
                 (compression_id, session_id, agent_id, source_start_message_id, source_end_message_id, summary_text, kept_message_count, trigger_reason, estimated_tokens_before, estimated_tokens_after, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    compression_id,
                    session_id,
                    agent_id,
                    source_start_message_id,
                    source_end_message_id,
                    summary_text,
                    kept_message_count,
                    trigger_reason,
                    estimated_tokens_before,
                    estimated_tokens_after,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "写入压缩记录失败，session_id：{session_id}，原因：{error}"
                ))
            })?;
        self.get_by_id(&compression_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("压缩记录未找到：{compression_id}")))
    }

    pub fn get_by_id(
        &self,
        compression_id: &str,
    ) -> Result<Option<ContextCompressionRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT compression_id, session_id, agent_id, source_start_message_id, source_end_message_id, summary_text, kept_message_count, trigger_reason, estimated_tokens_before, estimated_tokens_after, created_at
                 FROM context_compressions WHERE compression_id = ?1 LIMIT 1",
                params![compression_id],
                |row| {
                    Ok(ContextCompressionRecord {
                        compression_id: row.get(0)?,
                        session_id: row.get(1)?,
                        agent_id: row.get(2)?,
                        source_start_message_id: row.get(3)?,
                        source_end_message_id: row.get(4)?,
                        summary_text: row.get(5)?,
                        kept_message_count: row.get(6)?,
                        trigger_reason: row.get(7)?,
                        estimated_tokens_before: row.get(8)?,
                        estimated_tokens_after: row.get(9)?,
                        created_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询压缩记录失败，compression_id：{compression_id}，原因：{error}"
                ))
            })
    }

    pub fn list_by_session_id(
        &self,
        session_id: &str,
    ) -> Result<Vec<ContextCompressionRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT compression_id, session_id, agent_id, source_start_message_id, source_end_message_id, summary_text, kept_message_count, trigger_reason, estimated_tokens_before, estimated_tokens_after, created_at
                 FROM context_compressions WHERE session_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "准备压缩记录查询失败，session_id：{session_id}，原因：{error}"
                ))
            })?;
        let rows = statement
            .query_map(params![session_id], |row| {
                Ok(ContextCompressionRecord {
                    compression_id: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    source_start_message_id: row.get(3)?,
                    source_end_message_id: row.get(4)?,
                    summary_text: row.get(5)?,
                    kept_message_count: row.get(6)?,
                    trigger_reason: row.get(7)?,
                    estimated_tokens_before: row.get(8)?,
                    estimated_tokens_after: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询压缩记录失败，session_id：{session_id}，原因：{error}"
                ))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            RepositoryError::QueryFailed(format!(
                "读取压缩记录结果失败，session_id：{session_id}，原因：{error}"
            ))
        })
    }
}

/// 智能体仓储。
pub struct AgentRepository<'a> {
    connection: &'a Connection,
}

impl<'a> AgentRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn create_primary_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentRecord, RepositoryError> {
        let now = SessionRepository::current_timestamp();
        self.connection
            .execute(
                "INSERT INTO agents
                 (agent_id, session_id, parent_agent_id, agent_mode, depth, status, thread_key, task_summary, created_at, updated_at)
                 VALUES (?1, ?2, NULL, 'primary', 0, 'waiting', ?3, NULL, ?4, ?4)",
                params![agent_id, session_id, format!("THREAD-{agent_id}"), now],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建主智能体失败，agent_id：{agent_id}，原因：{error}"
                ))
            })?;
        self.get_by_id(agent_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("主智能体创建后未找到：{agent_id}")))
    }

    pub fn create_child_agent(
        &self,
        session_id: &str,
        parent_agent_id: &str,
        mode: &str,
        depth: i64,
        status: &str,
        task_summary: &str,
    ) -> Result<AgentRecord, RepositoryError> {
        let agent_id =
            WorkspaceRepository::next_identifier(self.connection, "agents", "agent_id", "AGT")?;
        let now = SessionRepository::current_timestamp();
        self.connection
            .execute(
                "INSERT INTO agents
                 (agent_id, session_id, parent_agent_id, agent_mode, depth, status, thread_key, task_summary, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    agent_id,
                    session_id,
                    parent_agent_id,
                    mode,
                    depth,
                    status,
                    format!("THREAD-{agent_id}"),
                    task_summary,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建子智能体失败，parent_agent_id：{parent_agent_id}，原因：{error}"
                ))
            })?;
        self.get_by_id(&agent_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("子智能体创建后未找到：{agent_id}")))
    }

    pub fn get_by_id(&self, agent_id: &str) -> Result<Option<AgentRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT agent_id, session_id, parent_agent_id, agent_mode, depth, status, thread_key, task_summary, created_at, updated_at
                 FROM agents WHERE agent_id = ?1 LIMIT 1",
                params![agent_id],
                |row| {
                    Ok(AgentRecord {
                        agent_id: row.get(0)?,
                        session_id: row.get(1)?,
                        parent_agent_id: row.get(2)?,
                        agent_mode: row.get(3)?,
                        depth: row.get(4)?,
                        status: row.get(5)?,
                        thread_key: row.get(6)?,
                        task_summary: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询智能体失败，agent_id：{agent_id}，原因：{error}"
                ))
            })
    }

    pub fn update_status_and_task(
        &self,
        agent_id: &str,
        status: &str,
        task_summary: &str,
    ) -> Result<AgentRecord, RepositoryError> {
        let now = SessionRepository::current_timestamp();
        let changed_rows = self
            .connection
            .execute(
                "UPDATE agents
                 SET status = ?1, task_summary = ?2, updated_at = ?3
                 WHERE agent_id = ?4",
                params![status, task_summary, now, agent_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新智能体状态失败，agent_id：{agent_id}，原因：{error}"
                ))
            })?;
        if changed_rows == 0 {
            return Err(RepositoryError::NotFound(format!(
                "智能体不存在：{agent_id}"
            )));
        }
        self.get_by_id(agent_id)?
            .ok_or_else(|| RepositoryError::NotFound(format!("智能体不存在：{agent_id}")))
    }

    pub fn count_active_children(&self) -> Result<i64, RepositoryError> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM agents
                 WHERE agent_mode != 'primary' AND status != 'destroyed'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!("统计活跃子智能体失败，原因：{error}"))
            })
    }
}

/// 智能体关系仓储。
pub struct AgentRelationRepository<'a> {
    connection: &'a Connection,
}

impl<'a> AgentRelationRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn create(
        &self,
        parent_agent_id: &str,
        child_agent_id: &str,
        relation_mode: &str,
        handoff_summary: Option<String>,
        result_summary: Option<String>,
    ) -> Result<AgentRelationRecord, RepositoryError> {
        let relation_id = WorkspaceRepository::next_identifier(
            self.connection,
            "agent_relations",
            "relation_id",
            "REL",
        )?;
        let now = SessionRepository::current_timestamp();
        self.connection
            .execute(
                "INSERT INTO agent_relations
                 (relation_id, parent_agent_id, child_agent_id, relation_mode, handoff_summary, result_summary, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    relation_id,
                    parent_agent_id,
                    child_agent_id,
                    relation_mode,
                    handoff_summary,
                    result_summary,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建智能体关系失败，child_agent_id：{child_agent_id}，原因：{error}"
                ))
            })?;
        self.get_by_child_agent(child_agent_id)?.ok_or_else(|| {
            RepositoryError::NotFound(format!("智能体关系创建后未找到：{child_agent_id}"))
        })
    }

    pub fn get_by_child_agent(
        &self,
        child_agent_id: &str,
    ) -> Result<Option<AgentRelationRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT relation_id, parent_agent_id, child_agent_id, relation_mode, handoff_summary, result_summary, created_at
                 FROM agent_relations WHERE child_agent_id = ?1 LIMIT 1",
                params![child_agent_id],
                |row| {
                    Ok(AgentRelationRecord {
                        relation_id: row.get(0)?,
                        parent_agent_id: row.get(1)?,
                        child_agent_id: row.get(2)?,
                        relation_mode: row.get(3)?,
                        handoff_summary: row.get(4)?,
                        result_summary: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询智能体关系失败，child_agent_id：{child_agent_id}，原因：{error}"
                ))
            })
    }

    pub fn list_by_parent_agent(
        &self,
        parent_agent_id: &str,
    ) -> Result<Vec<AgentRelationRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT relation_id, parent_agent_id, child_agent_id, relation_mode, handoff_summary, result_summary, created_at
                 FROM agent_relations WHERE parent_agent_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "准备父子关系查询失败，parent_agent_id：{parent_agent_id}，原因：{error}"
                ))
            })?;
        let rows = statement
            .query_map(params![parent_agent_id], |row| {
                Ok(AgentRelationRecord {
                    relation_id: row.get(0)?,
                    parent_agent_id: row.get(1)?,
                    child_agent_id: row.get(2)?,
                    relation_mode: row.get(3)?,
                    handoff_summary: row.get(4)?,
                    result_summary: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询父子关系失败，parent_agent_id：{parent_agent_id}，原因：{error}"
                ))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            RepositoryError::QueryFailed(format!(
                "读取父子关系结果失败，parent_agent_id：{parent_agent_id}，原因：{error}"
            ))
        })
    }

    pub fn update_result_summary(
        &self,
        relation_id: &str,
        result_summary: Option<String>,
    ) -> Result<(), RepositoryError> {
        self.connection
            .execute(
                "UPDATE agent_relations SET result_summary = ?1 WHERE relation_id = ?2",
                params![result_summary, relation_id],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "更新关系结果摘要失败，relation_id：{relation_id}，原因：{error}"
                ))
            })?;
        Ok(())
    }
}

/// 事件日志插入输入。
pub struct EventLogInsertInput {
    pub event_id: String,
    pub round_id: Option<String>,
    pub source_session_id: Option<String>,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub target_agent_id: Option<String>,
    pub event_type: String,
    pub payload_summary: String,
    pub status: String,
}

/// 事件日志仓储。
pub struct EventLogRepository<'a> {
    connection: &'a Connection,
}

impl<'a> EventLogRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn next_event_id(&self) -> Result<String, RepositoryError> {
        WorkspaceRepository::next_identifier(self.connection, "event_logs", "event_id", "EVT")
    }

    pub fn insert(&self, input: EventLogInsertInput) -> Result<EventLogRecord, RepositoryError> {
        let now = SessionRepository::current_timestamp();
        self.connection
            .execute(
                "INSERT INTO event_logs
                 (event_id, round_id, source_session_id, session_id, agent_id, target_agent_id, event_type, payload_summary, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    input.event_id,
                    input.round_id,
                    input.source_session_id,
                    input.session_id,
                    input.agent_id,
                    input.target_agent_id,
                    input.event_type,
                    input.payload_summary,
                    input.status,
                    now
                ],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!("写入事件日志失败，原因：{error}"))
            })?;
        self.get_by_id(&input.event_id)?.ok_or_else(|| {
            RepositoryError::NotFound(format!("事件日志创建后未找到：{}", input.event_id))
        })
    }

    pub fn get_by_id(&self, event_id: &str) -> Result<Option<EventLogRecord>, RepositoryError> {
        self.connection
            .query_row(
                "SELECT event_id, round_id, source_session_id, session_id, agent_id, target_agent_id, event_type, payload_summary, status, created_at
                 FROM event_logs WHERE event_id = ?1 LIMIT 1",
                params![event_id],
                |row| {
                    Ok(EventLogRecord {
                        event_id: row.get(0)?,
                        round_id: row.get(1)?,
                        source_session_id: row.get(2)?,
                        session_id: row.get(3)?,
                        agent_id: row.get(4)?,
                        target_agent_id: row.get(5)?,
                        event_type: row.get(6)?,
                        payload_summary: row.get(7)?,
                        status: row.get(8)?,
                        created_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询事件日志失败，event_id：{event_id}，原因：{error}"
                ))
            })
    }

    pub fn list_by_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<EventLogRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT event_id, round_id, source_session_id, session_id, agent_id, target_agent_id, event_type, payload_summary, status, created_at
                 FROM event_logs WHERE session_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "准备事件日志查询失败，session_id：{session_id}，原因：{error}"
                ))
            })?;
        let rows = statement
            .query_map(params![session_id], |row| {
                Ok(EventLogRecord {
                    event_id: row.get(0)?,
                    round_id: row.get(1)?,
                    source_session_id: row.get(2)?,
                    session_id: row.get(3)?,
                    agent_id: row.get(4)?,
                    target_agent_id: row.get(5)?,
                    event_type: row.get(6)?,
                    payload_summary: row.get(7)?,
                    status: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "查询事件日志失败，session_id：{session_id}，原因：{error}"
                ))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            RepositoryError::QueryFailed(format!(
                "读取事件日志结果失败，session_id：{session_id}，原因：{error}"
            ))
        })
    }
}
