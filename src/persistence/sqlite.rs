//! SQLite 存储实现。

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::domain::{
    AgentInstance, ApprovalMode, DeletionAudit, Message, MessageRole, Session, SessionStatus,
    ToolCallRecord, ToolResultRecord, WorkingMemoryEntry, WorkspaceDirectory,
};

/// SQLite 存储封装。
pub struct SqliteStore {
    /// 线程内共享连接。
    connection: Mutex<Connection>,
}

impl SqliteStore {
    /// 打开数据库并初始化表结构。
    pub fn new(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)
            .with_context(|| format!("打开数据库失败：{}", path.display()))?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.initialize()?;
        Ok(store)
    }

    /// 初始化当前阶段需要的表结构。
    fn initialize(&self) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS workspace_directory (
                id TEXT PRIMARY KEY,
                project_name TEXT NOT NULL,
                project_path TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                is_deleted INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS session (
                id TEXT PRIMARY KEY,
                directory_id TEXT NOT NULL,
                name TEXT NOT NULL,
                project_name TEXT NOT NULL,
                project_path TEXT NOT NULL,
                working_directory TEXT NOT NULL,
                model TEXT NOT NULL,
                approval_mode TEXT NOT NULL,
                status TEXT NOT NULL,
                stream_output INTEGER NOT NULL,
                round INTEGER NOT NULL,
                is_finished INTEGER NOT NULL,
                session_dir TEXT NOT NULL,
                snapshot_version INTEGER NOT NULL,
                last_round_no INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                system_prompt TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS message (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                name TEXT,
                tool_call_id TEXT,
                tool_calls_json TEXT,
                reasoning_content TEXT,
                metadata_json TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS deletion_audit (
                id TEXT PRIMARY KEY,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                delete_mode TEXT NOT NULL,
                artifact_missing INTEGER NOT NULL,
                operator TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                restored_at TEXT
            );

            CREATE TABLE IF NOT EXISTS tool_call (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                round_no INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                arguments_json TEXT NOT NULL,
                status TEXT NOT NULL,
                success INTEGER NOT NULL,
                failure_type TEXT,
                error_message TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tool_result_index (
                tool_call_id TEXT PRIMARY KEY,
                tool_name TEXT NOT NULL,
                handle TEXT NOT NULL,
                body_file_path TEXT NOT NULL,
                projection_type TEXT NOT NULL,
                projection_content TEXT NOT NULL,
                summary TEXT NOT NULL,
                preview_head TEXT NOT NULL DEFAULT '',
                preview_tail TEXT NOT NULL DEFAULT '',
                char_count INTEGER NOT NULL,
                byte_count INTEGER NOT NULL DEFAULT 0,
                success INTEGER NOT NULL,
                truncated INTEGER NOT NULL,
                externalized INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS working_memory (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                compact_boundary_id TEXT NOT NULL,
                working_memory_version INTEGER NOT NULL,
                estimated_tokens_before INTEGER NOT NULL,
                estimated_tokens_after INTEGER NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agent_instance (
                id TEXT PRIMARY KEY,
                parent_session_id TEXT NOT NULL,
                parent_agent_id TEXT,
                mode TEXT NOT NULL,
                inherit_context INTEGER NOT NULL,
                level INTEGER NOT NULL,
                status TEXT NOT NULL,
                session_dir TEXT NOT NULL,
                child_session_id TEXT NOT NULL,
                allowed_paths_json TEXT NOT NULL,
                task_spec_json TEXT NOT NULL,
                constraint_hash TEXT NOT NULL,
                result_summary TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )?;
        let _ = connection.execute(
            "ALTER TABLE tool_result_index ADD COLUMN preview_head TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = connection.execute(
            "ALTER TABLE tool_result_index ADD COLUMN preview_tail TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = connection.execute(
            "ALTER TABLE tool_result_index ADD COLUMN byte_count INTEGER NOT NULL DEFAULT 0",
            [],
        );
        Ok(())
    }

    /// 写入或更新工作区记录。
    pub fn upsert_workspace(&self, workspace: &WorkspaceDirectory) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT INTO workspace_directory (id, project_name, project_path, created_at, updated_at, is_deleted)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                project_name = excluded.project_name,
                project_path = excluded.project_path,
                updated_at = excluded.updated_at,
                is_deleted = excluded.is_deleted
            "#,
            params![
                workspace.id,
                workspace.project_name,
                workspace.project_path,
                workspace.created_at,
                workspace.updated_at,
                i64::from(workspace.is_deleted)
            ],
        )?;
        Ok(())
    }

    /// 写入或更新会话记录。
    pub fn upsert_session(&self, session: &Session) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT INTO session (
                id, directory_id, name, project_name, project_path, working_directory,
                model, approval_mode, status, stream_output, round, is_finished, session_dir,
                snapshot_version, last_round_no, content_hash, system_prompt, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                project_name = excluded.project_name,
                project_path = excluded.project_path,
                working_directory = excluded.working_directory,
                model = excluded.model,
                approval_mode = excluded.approval_mode,
                status = excluded.status,
                stream_output = excluded.stream_output,
                round = excluded.round,
                is_finished = excluded.is_finished,
                session_dir = excluded.session_dir,
                snapshot_version = excluded.snapshot_version,
                last_round_no = excluded.last_round_no,
                content_hash = excluded.content_hash,
                system_prompt = excluded.system_prompt,
                updated_at = excluded.updated_at
            "#,
            params![
                session.id,
                session.directory_id,
                session.name,
                session.project_name,
                session.project_path,
                session.working_directory,
                session.model,
                session.approval_mode.as_str(),
                session.status.as_str(),
                i64::from(session.stream_output),
                session.round,
                i64::from(session.is_finished),
                session.session_dir.to_string_lossy().to_string(),
                session.snapshot_version,
                session.last_round_no,
                session.content_hash,
                session.system_prompt,
                session.created_at,
                session.updated_at
            ],
        )?;
        Ok(())
    }

    /// 使用完整快照替换会话的消息集合。
    pub fn replace_messages(&self, session_id: &str, messages: &[Message]) -> Result<()> {
        let mut connection = self.connection.lock().expect("数据库锁已中毒");
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM message WHERE session_id = ?1",
            params![session_id],
        )?;

        for message in messages {
            let role = match message.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
            };
            let tool_calls_json = if message.tool_calls.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&message.tool_calls)?)
            };
            let metadata_json = message
                .metadata
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;

            transaction.execute(
                r#"
                INSERT INTO message (
                    session_id, role, content, name, tool_call_id, tool_calls_json, reasoning_content, metadata_json, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
                "#,
                params![
                    session_id,
                    role,
                    message.content,
                    message.name,
                    message.tool_call_id,
                    tool_calls_json,
                    message.reasoning_content,
                    metadata_json
                ],
            )?;
        }

        transaction.commit()?;
        Ok(())
    }

    /// 按会话读取消息列表。
    pub fn list_messages_by_session(&self, session_id: &str) -> Result<Vec<Message>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT role, content, name, tool_call_id, tool_calls_json, reasoning_content, metadata_json
            FROM message
            WHERE session_id = ?1
            ORDER BY id ASC
            "#,
        )?;

        let rows = statement.query_map(params![session_id], |row| {
            let role = match row.get::<_, String>(0)?.as_str() {
                "system" => MessageRole::System,
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                _ => MessageRole::User,
            };
            let tool_calls_json: Option<String> = row.get(4)?;
            let metadata_json: Option<String> = row.get(6)?;

            Ok(Message {
                role,
                content: row.get(1)?,
                name: row.get(2)?,
                tool_call_id: row.get(3)?,
                tool_calls: tool_calls_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .unwrap_or_default()
                    .unwrap_or_default(),
                reasoning_content: row.get(5)?,
                metadata: metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .unwrap_or_default()
                    .unwrap_or(None),
            })
        })?;

        let messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    /// 读取全部会话列表。
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT
                id, directory_id, name, project_name, project_path, working_directory,
                model, approval_mode, status, stream_output, round, is_finished,
                session_dir, snapshot_version, last_round_no, content_hash, system_prompt,
                created_at, updated_at
            FROM session
            ORDER BY updated_at DESC
            "#,
        )?;

        let rows = statement.query_map([], |row| self.row_to_session(row))?;
        let sessions = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(sessions)
    }

    /// 按会话标识或名称查询会话。
    pub fn find_session(&self, key: &str) -> Result<Option<Session>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT
                id, directory_id, name, project_name, project_path, working_directory,
                model, approval_mode, status, stream_output, round, is_finished,
                session_dir, snapshot_version, last_round_no, content_hash, system_prompt,
                created_at, updated_at
            FROM session
            WHERE id = ?1 OR name = ?1
            LIMIT 1
            "#,
        )?;

        let session = statement
            .query_row(params![key], |row| self.row_to_session(row))
            .optional()?;
        Ok(session)
    }

    /// 按目录标识列出会话。
    pub fn list_sessions_by_directory(&self, directory_id: &str) -> Result<Vec<Session>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT
                id, directory_id, name, project_name, project_path, working_directory,
                model, approval_mode, status, stream_output, round, is_finished,
                session_dir, snapshot_version, last_round_no, content_hash, system_prompt,
                created_at, updated_at
            FROM session
            WHERE directory_id = ?1
            ORDER BY updated_at DESC
            "#,
        )?;

        let rows = statement.query_map(params![directory_id], |row| self.row_to_session(row))?;
        let sessions = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(sessions)
    }

    /// 读取工作区列表。
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceDirectory>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT id, project_name, project_path, created_at, updated_at, is_deleted
            FROM workspace_directory
            ORDER BY updated_at DESC
            "#,
        )?;

        let rows = statement.query_map([], |row| self.row_to_workspace(row))?;
        let workspaces = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(workspaces)
    }

    /// 按标识、名称或路径查询工作区。
    pub fn find_workspace(&self, key: &str) -> Result<Option<WorkspaceDirectory>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT id, project_name, project_path, created_at, updated_at, is_deleted
            FROM workspace_directory
            WHERE id = ?1 OR project_name = ?1 OR project_path = ?1
            LIMIT 1
            "#,
        )?;

        let workspace = statement
            .query_row(params![key], |row| self.row_to_workspace(row))
            .optional()?;
        Ok(workspace)
    }

    /// 删除会话元数据与消息记录。
    pub fn delete_session_records(&self, session_id: &str) -> Result<()> {
        let mut connection = self.connection.lock().expect("数据库锁已中毒");
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM message WHERE session_id = ?1",
            params![session_id],
        )?;
        transaction.execute("DELETE FROM session WHERE id = ?1", params![session_id])?;
        transaction.commit()?;
        Ok(())
    }

    /// 删除工作区元数据。
    pub fn delete_workspace_record(&self, workspace_id: &str) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            "DELETE FROM workspace_directory WHERE id = ?1",
            params![workspace_id],
        )?;
        Ok(())
    }

    /// 写入删除审计记录。
    pub fn insert_deletion_audit(&self, audit: &DeletionAudit) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT INTO deletion_audit (
                id, target_type, target_id, delete_mode, artifact_missing, operator, payload_json, created_at, restored_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                audit.id,
                audit.target_type,
                audit.target_id,
                audit.delete_mode,
                i64::from(audit.artifact_missing),
                audit.operator,
                audit.payload_json,
                audit.created_at,
                audit.restored_at
            ],
        )?;
        Ok(())
    }

    /// 写入工具调用审计记录。
    pub fn insert_tool_call(&self, record: &ToolCallRecord) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT OR REPLACE INTO tool_call (
                id, session_id, round_no, tool_name, arguments_json, status, success,
                failure_type, error_message, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                record.id,
                record.session_id,
                record.round_no,
                record.tool_name,
                record.arguments_json,
                record.status,
                i64::from(record.success),
                record
                    .failure_type
                    .as_ref()
                    .map(|value| format!("{:?}", value)),
                record.error_message,
                record.created_at,
                record.updated_at
            ],
        )?;
        Ok(())
    }

    /// 列出指定会话的工具调用记录。
    pub fn list_tool_calls(&self, session_id: &str) -> Result<Vec<ToolCallRecord>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT id, session_id, round_no, tool_name, arguments_json, status, success,
                   failure_type, error_message, created_at, updated_at
            FROM tool_call
            WHERE session_id = ?1
            ORDER BY created_at DESC
            "#,
        )?;

        let rows = statement.query_map(params![session_id], |row| {
            Ok(ToolCallRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                round_no: row.get(2)?,
                tool_name: row.get(3)?,
                arguments_json: row.get(4)?,
                status: row.get(5)?,
                success: row.get::<_, i64>(6)? != 0,
                failure_type: match row.get::<_, Option<String>>(7)? {
                    Some(value) if value == "InvalidArgs" => {
                        Some(crate::domain::ToolFailureType::InvalidArgs)
                    }
                    Some(value) if value == "ApprovalDenied" => {
                        Some(crate::domain::ToolFailureType::ApprovalDenied)
                    }
                    Some(value) if value == "ExecError" => {
                        Some(crate::domain::ToolFailureType::ExecError)
                    }
                    _ => None,
                },
                error_message: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;

        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(items)
    }

    /// 写入工具结果索引记录。
    pub fn insert_tool_result_index(&self, record: &ToolResultRecord) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT OR REPLACE INTO tool_result_index (
                tool_call_id, tool_name, handle, body_file_path, projection_type,
                projection_content, summary, preview_head, preview_tail, char_count, byte_count,
                success, truncated, externalized, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
            params![
                record.tool_call_id,
                record.tool_name,
                record.handle,
                record.body_file_path,
                format!("{:?}", record.projection_type),
                record.projection_content,
                record.summary,
                record.preview_head,
                record.preview_tail,
                record.char_count as i64,
                record.byte_count as i64,
                i64::from(record.success),
                i64::from(record.truncated),
                i64::from(record.externalized),
                record.updated_at
            ],
        )?;
        Ok(())
    }

    /// 列出指定会话的工具结果索引。
    pub fn list_tool_result_indexes(&self, session_id: &str) -> Result<Vec<ToolResultRecord>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT tri.tool_call_id, tri.tool_name, tri.handle, tri.body_file_path,
                   tri.projection_type, tri.projection_content, tri.summary,
                   tri.preview_head, tri.preview_tail, tri.char_count, tri.byte_count,
                   tri.success, tri.truncated, tri.externalized, tri.updated_at
            FROM tool_result_index tri
            JOIN tool_call tc ON tc.id = tri.tool_call_id
            WHERE tc.session_id = ?1
            ORDER BY tri.updated_at DESC
            "#,
        )?;

        let rows = statement.query_map(params![session_id], |row| {
            Ok(ToolResultRecord {
                tool_call_id: row.get(0)?,
                tool_name: row.get(1)?,
                handle: row.get(2)?,
                body_file_path: row.get(3)?,
                projection_type: match row.get::<_, String>(4)?.as_str() {
                    "Summary" => crate::domain::ToolProjectionType::Summary,
                    _ => crate::domain::ToolProjectionType::InlineFull,
                },
                projection_content: row.get(5)?,
                summary: row.get(6)?,
                preview_head: row.get(7)?,
                preview_tail: row.get(8)?,
                char_count: row.get::<_, i64>(9)? as usize,
                byte_count: row.get::<_, i64>(10)? as usize,
                success: row.get::<_, i64>(11)? != 0,
                truncated: row.get::<_, i64>(12)? != 0,
                externalized: row.get::<_, i64>(13)? != 0,
                updated_at: row.get(14)?,
            })
        })?;

        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(items)
    }

    /// 按工具调用标识查询单条工具结果索引。
    pub fn find_tool_result_index(&self, tool_call_id: &str) -> Result<Option<ToolResultRecord>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT tool_call_id, tool_name, handle, body_file_path, projection_type,
                   projection_content, summary, preview_head, preview_tail, char_count,
                   byte_count, success, truncated, externalized, updated_at
            FROM tool_result_index
            WHERE tool_call_id = ?1
            LIMIT 1
            "#,
        )?;

        let item = statement
            .query_row(params![tool_call_id], |row| {
                Ok(ToolResultRecord {
                    tool_call_id: row.get(0)?,
                    tool_name: row.get(1)?,
                    handle: row.get(2)?,
                    body_file_path: row.get(3)?,
                    projection_type: match row.get::<_, String>(4)?.as_str() {
                        "Summary" => crate::domain::ToolProjectionType::Summary,
                        _ => crate::domain::ToolProjectionType::InlineFull,
                    },
                    projection_content: row.get(5)?,
                    summary: row.get(6)?,
                    preview_head: row.get(7)?,
                    preview_tail: row.get(8)?,
                    char_count: row.get::<_, i64>(9)? as usize,
                    byte_count: row.get::<_, i64>(10)? as usize,
                    success: row.get::<_, i64>(11)? != 0,
                    truncated: row.get::<_, i64>(12)? != 0,
                    externalized: row.get::<_, i64>(13)? != 0,
                    updated_at: row.get(14)?,
                })
            })
            .optional()?;
        Ok(item)
    }

    /// 写入工作记忆记录。
    pub fn insert_working_memory(
        &self,
        id: &str,
        session_id: &str,
        compact_boundary_id: &str,
        working_memory_version: i64,
        estimated_tokens_before: i64,
        estimated_tokens_after: i64,
        content: &str,
        created_at: &str,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT OR REPLACE INTO working_memory (
                id, session_id, compact_boundary_id, working_memory_version,
                estimated_tokens_before, estimated_tokens_after, content, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                id,
                session_id,
                compact_boundary_id,
                working_memory_version,
                estimated_tokens_before,
                estimated_tokens_after,
                content,
                created_at
            ],
        )?;
        Ok(())
    }

    /// 写入子 Agent 记录。
    pub fn upsert_agent_instance(&self, agent: &AgentInstance) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            r#"
            INSERT OR REPLACE INTO agent_instance (
                id, parent_session_id, parent_agent_id, mode, inherit_context, level,
                status, session_dir, child_session_id, allowed_paths_json, task_spec_json,
                constraint_hash, result_summary, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
            params![
                agent.id,
                agent.parent_session_id,
                agent.parent_agent_id,
                format!("{:?}", agent.mode),
                i64::from(agent.inherit_context),
                agent.level,
                format!("{:?}", agent.status),
                agent.session_dir,
                agent.child_session_id,
                agent.allowed_paths_json,
                agent.task_spec_json,
                agent.constraint_hash,
                agent.result_summary,
                agent.created_at,
                agent.updated_at
            ],
        )?;
        Ok(())
    }

    /// 列出指定会话的工作记忆。
    pub fn list_working_memories(&self, session_id: &str) -> Result<Vec<WorkingMemoryEntry>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT id, session_id, compact_boundary_id, working_memory_version,
                   estimated_tokens_before, estimated_tokens_after, content, created_at
            FROM working_memory
            WHERE session_id = ?1
            ORDER BY created_at DESC
            "#,
        )?;

        let rows = statement.query_map(params![session_id], |row| {
            Ok(WorkingMemoryEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                compact_boundary_id: row.get(2)?,
                working_memory_version: row.get(3)?,
                estimated_tokens_before: row.get(4)?,
                estimated_tokens_after: row.get(5)?,
                content: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;

        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(items)
    }

    /// 列出指定父会话的子 Agent。
    pub fn list_agent_instances(&self, parent_session_id: &str) -> Result<Vec<AgentInstance>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT id, parent_session_id, parent_agent_id, mode, inherit_context, level,
                   status, session_dir, child_session_id, allowed_paths_json, task_spec_json,
                   constraint_hash, result_summary, created_at, updated_at
            FROM agent_instance
            WHERE parent_session_id = ?1
            ORDER BY created_at DESC
            "#,
        )?;

        let rows = statement.query_map(params![parent_session_id], |row| {
            Ok(AgentInstance {
                id: row.get(0)?,
                parent_session_id: row.get(1)?,
                parent_agent_id: row.get(2)?,
                mode: match row.get::<_, String>(3)?.as_str() {
                    "Isolate" => crate::domain::AgentMode::Isolate,
                    _ => crate::domain::AgentMode::Inherit,
                },
                inherit_context: row.get::<_, i64>(4)? != 0,
                level: row.get(5)?,
                status: match row.get::<_, String>(6)?.as_str() {
                    "Running" => crate::domain::AgentStatus::Running,
                    "Done" => crate::domain::AgentStatus::Done,
                    "Closed" => crate::domain::AgentStatus::Closed,
                    _ => crate::domain::AgentStatus::Open,
                },
                session_dir: row.get(7)?,
                child_session_id: row.get(8)?,
                allowed_paths_json: row.get(9)?,
                task_spec_json: row.get(10)?,
                constraint_hash: row.get(11)?,
                result_summary: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
            })
        })?;

        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(items)
    }

    /// 查询指定类型的审计记录。
    pub fn find_deletion_audit(
        &self,
        target_type: &str,
        key: &str,
    ) -> Result<Option<DeletionAudit>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        let mut statement = connection.prepare(
            r#"
            SELECT id, target_type, target_id, delete_mode, artifact_missing, operator, payload_json, created_at, restored_at
            FROM deletion_audit
            WHERE target_type = ?1 AND (id = ?2 OR target_id = ?2)
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )?;

        let audit = statement
            .query_row(params![target_type, key], |row| self.row_to_audit(row))
            .optional()?;
        Ok(audit)
    }

    /// 列出审计记录。
    pub fn list_deletion_audits(&self, target_type: Option<&str>) -> Result<Vec<DeletionAudit>> {
        let connection = self.connection.lock().expect("数据库锁已中毒");

        let audits = if let Some(target_type) = target_type {
            let mut statement = connection.prepare(
                r#"
                SELECT id, target_type, target_id, delete_mode, artifact_missing, operator, payload_json, created_at, restored_at
                FROM deletion_audit
                WHERE target_type = ?1
                ORDER BY created_at DESC
                "#,
            )?;
            let rows = statement
                .query_map(params![target_type], |row| self.row_to_audit(row))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        } else {
            let mut statement = connection.prepare(
                r#"
                SELECT id, target_type, target_id, delete_mode, artifact_missing, operator, payload_json, created_at, restored_at
                FROM deletion_audit
                ORDER BY created_at DESC
                "#,
            )?;
            let rows = statement
                .query_map([], |row| self.row_to_audit(row))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };

        Ok(audits)
    }

    /// 标记审计记录已恢复。
    pub fn mark_deletion_audit_restored(&self, audit_id: &str, restored_at: &str) -> Result<()> {
        let connection = self.connection.lock().expect("数据库锁已中毒");
        connection.execute(
            "UPDATE deletion_audit SET restored_at = ?1 WHERE id = ?2",
            params![restored_at, audit_id],
        )?;
        Ok(())
    }

    /// 将查询行转换为会话实体。
    fn row_to_session(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
        Ok(Session {
            id: row.get(0)?,
            directory_id: row.get(1)?,
            name: row.get(2)?,
            project_name: row.get(3)?,
            project_path: row.get(4)?,
            working_directory: row.get(5)?,
            model: row.get(6)?,
            approval_mode: ApprovalMode::from_str(&row.get::<_, String>(7)?),
            status: SessionStatus::from_str(&row.get::<_, String>(8)?),
            stream_output: row.get::<_, i64>(9)? != 0,
            round: row.get(10)?,
            is_finished: row.get::<_, i64>(11)? != 0,
            session_dir: row.get::<_, String>(12)?.into(),
            snapshot_version: row.get(13)?,
            last_round_no: row.get(14)?,
            content_hash: row.get(15)?,
            system_prompt: row.get(16)?,
            created_at: row.get(17)?,
            updated_at: row.get(18)?,
        })
    }

    /// 将查询行转换为工作区实体。
    fn row_to_workspace(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceDirectory> {
        Ok(WorkspaceDirectory {
            id: row.get(0)?,
            project_name: row.get(1)?,
            project_path: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            is_deleted: row.get::<_, i64>(5)? != 0,
        })
    }

    /// 将查询行转换为审计实体。
    fn row_to_audit(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<DeletionAudit> {
        Ok(DeletionAudit {
            id: row.get(0)?,
            target_type: row.get(1)?,
            target_id: row.get(2)?,
            delete_mode: row.get(3)?,
            artifact_missing: row.get::<_, i64>(4)? != 0,
            operator: row.get(5)?,
            payload_json: row.get(6)?,
            created_at: row.get(7)?,
            restored_at: row.get(8)?,
        })
    }
}
