//! 仓储模块。
//!
//! 本模块负责 `workspaces`、`sessions`、`messages` 三类核心实体的
//! `SQLite` 持久化访问，实现 `TASK-005` 的基础仓储闭环。

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
    fn next_identifier(
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
        let message_id =
            WorkspaceRepository::next_identifier(self.connection, "messages", "message_id", "MSG")?;
        let sequence_no = self.next_sequence_no(session_id)?;
        let now = SessionRepository::current_timestamp();

        self.connection
            .execute(
                "INSERT INTO messages
                 (message_id, session_id, agent_id, round_id, sequence_no, role, content, content_type, token_estimate, include_in_context, is_compressed_source, created_at)
                 VALUES (?1, ?2, NULL, ?3, ?4, 'user', ?5, 'plain', 0, 1, 0, ?6)",
                params![message_id, session_id, round_id, sequence_no, content, now],
            )
            .map_err(|error| {
                RepositoryError::QueryFailed(format!(
                    "创建用户消息失败，session_id：{session_id}，原因：{error}"
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
                    "更新会话最后消息时间失败，session_id：{session_id}，原因：{error}"
                ))
            })?;

        self.list_by_session_id(session_id)?
            .into_iter()
            .find(|message| message.message_id == message_id)
            .ok_or_else(|| RepositoryError::NotFound(format!("消息创建后未找到：{message_id}")))
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
}
