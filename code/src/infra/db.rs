//! `SQLite` 数据库模块。
//!
//! 当前阶段负责数据库连接建立、迁移执行与启动自检，
//! 为后续仓储层实现提供稳定的数据基础设施。

use rusqlite::{Connection, OptionalExtension, params};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::PathBuf;

/// 数据库目标位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseTarget {
    /// 使用内存数据库，主要用于测试。
    InMemory,
    /// 使用文件数据库。
    File(PathBuf),
}

/// 数据库初始化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseBootstrapReport {
    /// 本次初始化所覆盖的核心表名列表。
    pub migrated_table_names: Vec<String>,
    /// 启动自检是否通过。
    pub self_check_passed: bool,
}

/// 数据库错误。
#[derive(Debug)]
pub enum DatabaseError {
    /// 创建数据库目录失败。
    DirectoryCreateFailed(String),
    /// 打开数据库失败。
    OpenFailed(String),
    /// 执行迁移失败。
    MigrationFailed(String),
    /// 启动自检失败。
    SelfCheckFailed(String),
    /// 查询数据库元信息失败。
    QueryFailed(String),
}

impl Display for DatabaseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DatabaseError::DirectoryCreateFailed(message) => write!(f, "{message}"),
            DatabaseError::OpenFailed(message) => write!(f, "{message}"),
            DatabaseError::MigrationFailed(message) => write!(f, "{message}"),
            DatabaseError::SelfCheckFailed(message) => write!(f, "{message}"),
            DatabaseError::QueryFailed(message) => write!(f, "{message}"),
        }
    }
}

impl Error for DatabaseError {}

/// `SQLite` 数据库访问入口。
pub struct SqliteDatabase {
    /// 当前数据库连接。
    connection: Connection,
}

impl SqliteDatabase {
    /// 打开指定目标的数据库连接。
    pub fn open(target: DatabaseTarget) -> Result<Self, DatabaseError> {
        let connection = match target {
            DatabaseTarget::InMemory => Connection::open_in_memory().map_err(|error| {
                DatabaseError::OpenFailed(format!("打开内存数据库失败：{error}"))
            })?,
            DatabaseTarget::File(path) => {
                Self::ensure_parent_directory(&path)?;
                Connection::open(&path).map_err(|error| {
                    DatabaseError::OpenFailed(format!(
                        "打开数据库文件失败，路径：{}，原因：{error}",
                        path.display()
                    ))
                })?
            }
        };

        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| {
                DatabaseError::OpenFailed(format!("启用 SQLite 外键约束失败：{error}"))
            })?;

        Ok(Self { connection })
    }

    /// 执行迁移并完成启动自检。
    pub fn initialize(&self) -> Result<DatabaseBootstrapReport, DatabaseError> {
        self.connection
            .execute_batch(Self::migration_sql())
            .map_err(|error| {
                DatabaseError::MigrationFailed(format!("执行数据库迁移失败：{error}"))
            })?;

        for table_name in Self::required_table_names() {
            if !self.table_exists(table_name)? {
                return Err(DatabaseError::SelfCheckFailed(format!(
                    "启动自检失败，缺少核心表：{table_name}"
                )));
            }
        }

        Ok(DatabaseBootstrapReport {
            migrated_table_names: Self::required_table_names()
                .iter()
                .map(|table_name| (*table_name).to_string())
                .collect(),
            self_check_passed: true,
        })
    }

    /// 判断指定表是否存在。
    pub fn table_exists(&self, table_name: &str) -> Result<bool, DatabaseError> {
        self.connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                params![table_name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|result| result.is_some())
            .map_err(|error| {
                DatabaseError::QueryFailed(format!(
                    "查询表是否存在失败，表名：{table_name}，原因：{error}"
                ))
            })
    }

    /// 返回只读数据库连接引用。
    ///
    /// 当前阶段仓储与服务通过该连接执行数据库访问。
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// 确保文件数据库的父目录存在。
    fn ensure_parent_directory(path: &PathBuf) -> Result<(), DatabaseError> {
        if let Some(parent_directory) = path.parent() {
            fs::create_dir_all(parent_directory).map_err(|error| {
                DatabaseError::DirectoryCreateFailed(format!(
                    "创建数据库目录失败，路径：{}，原因：{error}",
                    parent_directory.display()
                ))
            })?;
        }

        Ok(())
    }

    /// 返回系统要求的核心表名。
    fn required_table_names() -> &'static [&'static str] {
        &[
            "workspaces",
            "sessions",
            "agents",
            "agent_relations",
            "messages",
            "tool_calls",
            "context_compressions",
            "session_metrics",
            "event_logs",
        ]
    }

    /// 生成数据库迁移脚本。
    ///
    /// 当前阶段直接以内置 `SQL` 维护迁移基线，后续若迁移数量增多再拆分版本文件。
    fn migration_sql() -> &'static str {
        r#"
        CREATE TABLE IF NOT EXISTS workspaces (
            workspace_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            root_path TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            current_model TEXT NOT NULL,
            session_approval_mode TEXT NOT NULL,
            context_limit INTEGER NOT NULL,
            last_message_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (workspace_id) REFERENCES workspaces(workspace_id)
        );

        CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            parent_agent_id TEXT,
            agent_mode TEXT NOT NULL,
            depth INTEGER NOT NULL,
            status TEXT NOT NULL,
            thread_key TEXT NOT NULL UNIQUE,
            task_summary TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(session_id),
            FOREIGN KEY (parent_agent_id) REFERENCES agents(agent_id)
        );

        CREATE TABLE IF NOT EXISTS agent_relations (
            relation_id TEXT PRIMARY KEY,
            parent_agent_id TEXT NOT NULL,
            child_agent_id TEXT NOT NULL,
            relation_mode TEXT NOT NULL,
            handoff_summary TEXT,
            result_summary TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (parent_agent_id) REFERENCES agents(agent_id),
            FOREIGN KEY (child_agent_id) REFERENCES agents(agent_id)
        );

        CREATE TABLE IF NOT EXISTS messages (
            message_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_id TEXT,
            round_id TEXT NOT NULL,
            sequence_no INTEGER NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            content_type TEXT NOT NULL,
            token_estimate INTEGER NOT NULL DEFAULT 0,
            include_in_context INTEGER NOT NULL DEFAULT 1,
            is_compressed_source INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(session_id),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        );

        CREATE TABLE IF NOT EXISTS tool_calls (
            tool_call_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            round_id TEXT NOT NULL,
            arguments_json TEXT NOT NULL,
            session_approval_mode TEXT NOT NULL,
            tool_default_permission TEXT NOT NULL,
            visible INTEGER NOT NULL DEFAULT 1,
            background INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            exit_code INTEGER,
            result_summary TEXT,
            result_payload TEXT,
            error_code TEXT,
            failure_count_in_round INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(session_id),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        );

        CREATE TABLE IF NOT EXISTS context_compressions (
            compression_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            source_start_message_id TEXT NOT NULL,
            source_end_message_id TEXT NOT NULL,
            summary_text TEXT NOT NULL,
            kept_message_count INTEGER NOT NULL,
            trigger_reason TEXT NOT NULL,
            estimated_tokens_before INTEGER NOT NULL,
            estimated_tokens_after INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(session_id),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        );

        CREATE TABLE IF NOT EXISTS session_metrics (
            metric_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_id TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_hit_rate REAL NOT NULL DEFAULT 0,
            remaining_context INTEGER NOT NULL DEFAULT 0,
            tool_success_count INTEGER NOT NULL DEFAULT 0,
            tool_failure_count INTEGER NOT NULL DEFAULT 0,
            active_sessions INTEGER NOT NULL DEFAULT 0,
            active_child_agents INTEGER NOT NULL DEFAULT 0,
            active_tool_calls INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(session_id),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        );

        CREATE TABLE IF NOT EXISTS event_logs (
            event_id TEXT PRIMARY KEY,
            round_id TEXT,
            source_session_id TEXT,
            session_id TEXT NOT NULL,
            agent_id TEXT,
            target_agent_id TEXT,
            event_type TEXT NOT NULL,
            payload_summary TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(session_id),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_workspace_id
            ON sessions (workspace_id);

        CREATE INDEX IF NOT EXISTS idx_messages_session_sequence
            ON messages (session_id, sequence_no);

        CREATE INDEX IF NOT EXISTS idx_tool_calls_session_round
            ON tool_calls (session_id, round_id);

        CREATE INDEX IF NOT EXISTS idx_event_logs_session_round
            ON event_logs (session_id, round_id);
        "#
    }
}
