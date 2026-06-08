//! 会话管理器实现。

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::config::settings::Settings;
use crate::domain::{ApprovalMode, DeletionAudit, Message, Session, WorkspaceDirectory};
use crate::persistence::sqlite::SqliteStore;
use crate::session::snapshot::{
    ensure_tool_result_index, read_memory_json, read_session_ini, write_memory_json,
    write_session_ini, write_working_memory,
};
use crate::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use crate::utils::hash::sha256_hex;
use crate::utils::time::now_rfc3339;

/// 会话删除审计载荷。
#[derive(Debug, Serialize, Deserialize)]
struct SessionAuditPayload {
    /// 会话快照。
    session: Session,
    /// 会话消息快照。
    messages: Vec<Message>,
}

/// 工作区删除审计载荷。
#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceAuditPayload {
    /// 工作区快照。
    workspace: WorkspaceDirectory,
    /// 关联会话快照。
    sessions: Vec<SessionAuditPayload>,
}

/// 启动恢复报告。
#[derive(Debug, Default, Clone)]
pub struct RecoveryReport {
    /// 从文件恢复到数据库的会话数。
    pub restored_from_file: usize,
    /// 使用数据库覆盖旧文件快照的会话数。
    pub restored_from_database: usize,
    /// 使用文件覆盖数据库冲突的会话数。
    pub rebuilt_database_from_file: usize,
    /// 仅记录冲突但无法自动恢复的会话数。
    pub unresolved_conflicts: usize,
    /// 冲突日志列表。
    pub logs: Vec<String>,
}

/// 会话管理器。
pub struct SessionManager {
    /// 运行配置。
    settings: Settings,
    /// SQLite 存储。
    store: Arc<SqliteStore>,
}

impl SessionManager {
    /// 创建会话管理器。
    pub fn new(settings: Settings, store: Arc<SqliteStore>) -> Self {
        Self { settings, store }
    }

    /// 返回底层存储句柄。
    pub fn store(&self) -> Arc<SqliteStore> {
        self.store.clone()
    }

    /// 从快照文件修复数据库记录。
    pub fn repair_from_snapshots(&self) -> Result<RecoveryReport> {
        ensure_directory(&self.settings.sessions_root)?;
        let mut report = RecoveryReport::default();
        let mut snapshot_ids = HashSet::new();

        for entry in fs::read_dir(&self.settings.sessions_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let session_dir = entry.path();
            let session_ini = session_dir.join("session.ini");
            if !session_ini.exists() {
                continue;
            }

            match read_session_ini(&session_dir) {
                Ok(file_session) => {
                    snapshot_ids.insert(file_session.id.clone());
                    let file_messages = read_memory_json(&session_dir).unwrap_or_default();

                    match self.store.find_session(&file_session.id)? {
                        None => {
                            self.store
                                .upsert_workspace(&self.workspace_from_session(&file_session))?;
                            self.store.upsert_session(&file_session)?;
                            self.store
                                .replace_messages(&file_session.id, &file_messages)?;
                            report.restored_from_file += 1;
                            report.logs.push(format!(
                                "会话 {}：数据库缺失，已根据文件快照恢复",
                                file_session.id
                            ));
                        }
                        Some(db_session) => {
                            if file_session.last_round_no > db_session.last_round_no {
                                self.store.upsert_session(&file_session)?;
                                self.store
                                    .replace_messages(&file_session.id, &file_messages)?;
                                report.restored_from_file += 1;
                                report.logs.push(format!(
                                    "会话 {}：文件快照轮次 {} 大于数据库轮次 {}，已使用文件恢复数据库",
                                    file_session.id, file_session.last_round_no, db_session.last_round_no
                                ));
                            } else if file_session.last_round_no < db_session.last_round_no {
                                let db_messages =
                                    self.store.list_messages_by_session(&db_session.id)?;
                                self.save_snapshot(&db_session, &db_messages)?;
                                report.restored_from_database += 1;
                                report.logs.push(format!(
                                    "会话 {}：数据库轮次 {} 大于文件快照轮次 {}，已使用数据库回写文件快照",
                                    db_session.id, db_session.last_round_no, file_session.last_round_no
                                ));
                            } else if file_session.content_hash != db_session.content_hash {
                                self.store.upsert_session(&file_session)?;
                                self.store
                                    .replace_messages(&file_session.id, &file_messages)?;
                                report.rebuilt_database_from_file += 1;
                                report.logs.push(format!(
                                    "会话 {}：同轮次但 content_hash 不一致，已按文件快照重建数据库",
                                    file_session.id
                                ));
                            }
                        }
                    }
                }
                Err(error) => {
                    report.unresolved_conflicts += 1;
                    report.logs.push(format!(
                        "会话目录 {}：读取快照失败，原因：{}",
                        session_dir.display(),
                        error
                    ));
                    warn!(
                        "忽略损坏的会话快照：{}，原因：{}",
                        session_dir.display(),
                        error
                    );
                }
            }
        }

        for db_session in self.store.list_sessions()? {
            if snapshot_ids.contains(&db_session.id) {
                continue;
            }
            let session_ini = db_session.session_dir.join("session.ini");
            if !session_ini.exists() {
                report.unresolved_conflicts += 1;
                report.logs.push(format!(
                    "会话 {}：数据库存在，但文件快照缺失，未自动伪造快照",
                    db_session.id
                ));
            }
        }

        self.write_recovery_log(&report)?;
        Ok(report)
    }

    /// 创建新会话并初始化目录结构。
    pub fn create_session(
        &self,
        name: &str,
        model: &str,
        approval_mode: ApprovalMode,
        stream_output: bool,
        system_prompt: String,
    ) -> Result<Session> {
        let workspace = self.build_workspace_directory();
        self.store.upsert_workspace(&workspace)?;

        let session_id = Uuid::new_v4().to_string();
        let session_dir = self.settings.sessions_root.join(&session_id);
        self.initialize_session_directory(&session_dir)?;

        let mut session = Session::new(
            session_id,
            workspace.id.clone(),
            name.to_string(),
            workspace.project_name.clone(),
            workspace.project_path.clone(),
            self.settings.workspace_root.to_string_lossy().to_string(),
            model.to_string(),
            approval_mode,
            stream_output,
            session_dir,
            system_prompt,
        );
        session.content_hash = sha256_hex(&session.system_prompt);

        self.save_snapshot(&session, &[])?;
        Ok(session)
    }

    /// 保存会话完整快照。
    pub fn save_snapshot(&self, session: &Session, messages: &[Message]) -> Result<()> {
        write_session_ini(session)?;
        write_memory_json(&session.session_dir, messages)?;
        ensure_tool_result_index(&session.session_dir)?;
        self.store
            .upsert_workspace(&self.workspace_from_session(session))?;
        self.store.upsert_session(session)?;
        self.store.replace_messages(&session.id, messages)?;
        Ok(())
    }

    /// 加载会话消息列表。
    pub fn load_messages(&self, session: &Session) -> Result<Vec<Message>> {
        read_memory_json(&session.session_dir)
    }

    /// 写入工作记忆摘要。
    pub fn save_working_memory(
        &self,
        session: &Session,
        summary: &str,
        estimated_tokens_before: i64,
        estimated_tokens_after: i64,
    ) -> Result<()> {
        write_working_memory(&session.session_dir, summary)?;
        let created_at = now_rfc3339();
        self.store.insert_working_memory(
            &Uuid::new_v4().to_string(),
            &session.id,
            &format!("round-{}", session.round + 1),
            session.snapshot_version + 1,
            estimated_tokens_before,
            estimated_tokens_after,
            summary,
            &created_at,
        )
    }

    /// 列出某个会话的工作记忆。
    pub fn list_working_memories(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::domain::WorkingMemoryEntry>> {
        self.store.list_working_memories(session_id)
    }

    /// 列出某个会话的子 Agent。
    pub fn list_agent_instances(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::domain::AgentInstance>> {
        self.store.list_agent_instances(session_id)
    }

    /// 列出某个会话的工具调用记录。
    pub fn list_tool_calls(&self, session_id: &str) -> Result<Vec<crate::domain::ToolCallRecord>> {
        self.store.list_tool_calls(session_id)
    }

    /// 列出某个会话的工具结果索引。
    pub fn list_tool_result_indexes(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::domain::ToolResultRecord>> {
        let records = self.store.list_tool_result_indexes(session_id)?;
        if records.is_empty() {
            self.read_tool_result_index_from_file(session_id)
        } else {
            Ok(records)
        }
    }

    /// 按工具调用标识查询工具结果索引。
    pub fn find_tool_result_index(
        &self,
        tool_call_id: &str,
    ) -> Result<Option<crate::domain::ToolResultRecord>> {
        self.store.find_tool_result_index(tool_call_id)
    }

    /// 按句柄读取工具结果正文。
    pub fn read_tool_result_by_handle(&self, session_id: &str, handle: &str) -> Result<String> {
        let record = if let Some(record) = self
            .list_tool_result_indexes(session_id)?
            .into_iter()
            .find(|record| record.handle == handle)
        {
            record
        } else {
            self.read_tool_result_index_from_file(session_id)?
                .into_iter()
                .find(|record| record.handle == handle)
                .ok_or_else(|| anyhow!("未找到工具结果句柄：{}", handle))?
        };
        self.read_tool_result_record_body(&record)
    }

    /// 按工具调用标识读取工具结果正文。
    pub fn read_tool_result_by_call_id(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> Result<String> {
        let record = if let Some(record) = self.find_tool_result_index(tool_call_id)? {
            record
        } else {
            self.read_tool_result_index_from_file(session_id)?
                .into_iter()
                .find(|record| record.tool_call_id == tool_call_id)
                .ok_or_else(|| anyhow!("未找到工具结果：{}", tool_call_id))?
        };
        self.read_tool_result_record_body(&record)
    }

    /// 按会话名称或标识查询会话。
    pub fn use_session(&self, key: &str) -> Result<Session> {
        self.store
            .find_session(key)?
            .ok_or_else(|| anyhow!("未找到会话：{}", key))
    }

    /// 列出全部会话。
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        self.store.list_sessions()
    }

    /// 列出工作区元数据。
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceDirectory>> {
        self.store.list_workspaces()
    }

    /// 列出删除审计记录。
    pub fn list_deletion_audits(&self, target_type: Option<&str>) -> Result<Vec<DeletionAudit>> {
        self.store.list_deletion_audits(target_type)
    }

    /// 删除指定会话，并写入恢复审计快照。
    pub fn delete_session(&self, key: &str, operator: &str) -> Result<String> {
        let session = self.use_session(key)?;
        if session.status == crate::domain::SessionStatus::Running {
            return Err(anyhow!("当前会话正在运行中，禁止删除：{}", session.name));
        }
        let artifact_missing = !session.session_dir.exists();
        let messages = if artifact_missing {
            Vec::new()
        } else {
            read_memory_json(&session.session_dir)?
        };
        let payload = serde_json::to_string(&SessionAuditPayload {
            session: session.clone(),
            messages,
        })?;
        let audit = DeletionAudit::new(
            "session",
            session.id.clone(),
            "metadata_and_generated_artifacts",
            artifact_missing,
            operator,
            payload,
        );

        self.store.insert_deletion_audit(&audit)?;
        if session.session_dir.exists() {
            fs::remove_dir_all(&session.session_dir)?;
        }
        self.store.delete_session_records(&session.id)?;
        Ok(audit.id)
    }

    /// 从审计快照恢复会话。
    pub fn restore_session(&self, key: &str) -> Result<Session> {
        let audit = self
            .store
            .find_deletion_audit("session", key)?
            .ok_or_else(|| anyhow!("未找到会话恢复审计记录：{}", key))?;
        let payload: SessionAuditPayload = serde_json::from_str(&audit.payload_json)?;

        self.initialize_session_directory(&payload.session.session_dir)?;
        self.save_snapshot(&payload.session, &payload.messages)?;
        self.store
            .mark_deletion_audit_restored(&audit.id, &now_rfc3339())?;
        Ok(payload.session)
    }

    /// 删除工作区元数据、关联会话元数据及生成产物。
    pub fn delete_workspace(&self, key: &str, operator: &str) -> Result<String> {
        let workspace = self
            .store
            .find_workspace(key)?
            .ok_or_else(|| anyhow!("未找到工作区：{}", key))?;
        let sessions = self.store.list_sessions_by_directory(&workspace.id)?;
        if let Some(running_session) = sessions
            .iter()
            .find(|session| session.status == crate::domain::SessionStatus::Running)
        {
            return Err(anyhow!(
                "存在运行中的会话，禁止删除工作区：{}",
                running_session.name
            ));
        }

        let payload_sessions = sessions
            .iter()
            .map(|session| {
                let messages = if session.session_dir.exists() {
                    read_memory_json(&session.session_dir)
                } else {
                    Ok(Vec::new())
                }?;
                Ok(SessionAuditPayload {
                    session: session.clone(),
                    messages,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let payload = serde_json::to_string(&WorkspaceAuditPayload {
            workspace: workspace.clone(),
            sessions: payload_sessions,
        })?;
        let audit = DeletionAudit::new(
            "workspace_directory",
            workspace.id.clone(),
            "metadata_and_generated_artifacts",
            false,
            operator,
            payload,
        );

        self.store.insert_deletion_audit(&audit)?;
        for session in sessions {
            if session.session_dir.exists() {
                fs::remove_dir_all(&session.session_dir)?;
            }
            self.store.delete_session_records(&session.id)?;
        }
        self.store.delete_workspace_record(&workspace.id)?;
        Ok(audit.id)
    }

    /// 从审计快照恢复工作区及其关联会话。
    pub fn restore_workspace(&self, key: &str) -> Result<WorkspaceDirectory> {
        let audit = self
            .store
            .find_deletion_audit("workspace_directory", key)?
            .ok_or_else(|| anyhow!("未找到工作区恢复审计记录：{}", key))?;
        let payload: WorkspaceAuditPayload = serde_json::from_str(&audit.payload_json)?;

        self.store.upsert_workspace(&payload.workspace)?;
        for session_payload in payload.sessions {
            self.initialize_session_directory(&session_payload.session.session_dir)?;
            self.save_snapshot(&session_payload.session, &session_payload.messages)?;
        }
        self.store
            .mark_deletion_audit_restored(&audit.id, &now_rfc3339())?;
        Ok(payload.workspace)
    }

    /// 初始化会话目录结构。
    fn initialize_session_directory(&self, session_dir: &Path) -> Result<()> {
        ensure_directory(session_dir)?;
        ensure_directory(&session_dir.join("tool_results"))?;
        ensure_directory(&session_dir.join(".tools").join("task"))?;
        ensure_directory(&session_dir.join(".tools").join("automation"))?;
        ensure_directory(&session_dir.join(".tools").join("events"))?;
        ensure_directory(&session_dir.join(".tools").join("mcp"))?;
        ensure_directory(&session_dir.join(".tools").join("agent"))?;
        ensure_directory(&session_dir.join(".tools").join("plan"))?;
        ensure_directory(&session_dir.join(".tools").join("rlm"))?;
        ensure_json_file(&session_dir.join(".tools").join("task").join("tasks.json"))?;
        ensure_json_file(
            &session_dir
                .join(".tools")
                .join("automation")
                .join("automations.json"),
        )?;
        ensure_json_file(&session_dir.join(".tools").join("mcp").join("clients.json"))?;
        ensure_json_file(&session_dir.join(".tools").join("agent").join("agents.json"))?;
        ensure_json_file(
            &session_dir
                .join(".tools")
                .join("events")
                .join("events.json"),
        )?;
        Ok(())
    }

    /// 根据当前工作区构造主工作区实体。
    fn build_workspace_directory(&self) -> WorkspaceDirectory {
        let now = now_rfc3339();
        let project_path = self.settings.workspace_root.to_string_lossy().to_string();
        let project_name = self
            .settings
            .workspace_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string();

        WorkspaceDirectory {
            id: sha256_hex(&project_path),
            project_name,
            project_path,
            created_at: now.clone(),
            updated_at: now,
            is_deleted: false,
        }
    }

    /// 从会话快照反推工作区实体。
    fn workspace_from_session(&self, session: &Session) -> WorkspaceDirectory {
        WorkspaceDirectory {
            id: session.directory_id.clone(),
            project_name: session.project_name.clone(),
            project_path: session.project_path.clone(),
            created_at: session.created_at.clone(),
            updated_at: session.updated_at.clone(),
            is_deleted: false,
        }
    }

    fn read_tool_result_record_body(
        &self,
        record: &crate::domain::ToolResultRecord,
    ) -> Result<String> {
        if !record.body_file_path.is_empty() {
            read_optional_utf8(Path::new(&record.body_file_path))?
                .ok_or_else(|| anyhow!("工具结果正文不存在：{}", record.body_file_path))
        } else {
            Ok(record.projection_content.clone())
        }
    }

    fn read_tool_result_index_from_file(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::domain::ToolResultRecord>> {
        let session = self.use_session(session_id)?;
        let path = session.session_dir.join("tool_results").join("index.json");
        let content = read_optional_utf8(&path)?
            .ok_or_else(|| anyhow!("工具结果索引不存在：{}", path.display()))?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    /// 写入恢复日志。
    fn write_recovery_log(&self, report: &RecoveryReport) -> Result<()> {
        let path = self.settings.data_root.join("recovery.log");
        let content = if report.logs.is_empty() {
            format!("{} 启动恢复完成，未发现冲突。\n", now_rfc3339())
        } else {
            format!(
                "{} 启动恢复完成：\n{}\n",
                now_rfc3339(),
                report.logs.join("\n")
            )
        };
        write_utf8(&path, &content)
    }
}

fn ensure_json_file(path: &Path) -> Result<()> {
    if !path.exists() {
        write_utf8(path, "[]\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::config::settings::Settings;
    use crate::domain::{ApprovalMode, Message, Session};
    use crate::persistence::sqlite::SqliteStore;
    use crate::session::snapshot::{read_session_ini, write_memory_json, write_session_ini};

    use super::SessionManager;

    #[test]
    fn should_repair_session_from_snapshot_when_database_missing() {
        let workspace = PathBuf::from(format!(
            "target/test_repair_workspace_{}",
            uuid::Uuid::new_v4()
        ));
        let settings = Settings::load(&workspace).expect("加载测试配置失败");
        let store = Arc::new(SqliteStore::new(&settings.database_path).expect("创建数据库失败"));
        let manager = SessionManager::new(settings.clone(), store.clone());

        let session_dir = settings.sessions_root.join("session-demo");
        let session = Session::new(
            "session-demo".to_string(),
            "directory-demo".to_string(),
            "demo".to_string(),
            "demo-project".to_string(),
            workspace.to_string_lossy().to_string(),
            workspace.to_string_lossy().to_string(),
            "deepseek-v4-flash".to_string(),
            ApprovalMode::AskUser,
            true,
            session_dir.clone(),
            "system prompt".to_string(),
        );
        write_session_ini(&session).expect("写入 session.ini 失败");
        write_memory_json(&session_dir, &[Message::user("hello")]).expect("写入 memory.json 失败");

        let report = manager.repair_from_snapshots().expect("执行恢复失败");
        let sessions = manager.list_sessions().expect("读取会话列表失败");

        assert_eq!(report.restored_from_file, 1);
        assert!(sessions.iter().any(|item| item.id == "session-demo"));
    }

    #[test]
    fn should_prefer_database_when_database_round_is_newer() {
        let workspace = PathBuf::from(format!(
            "target/test_repair_db_newer_{}",
            uuid::Uuid::new_v4()
        ));
        let settings = Settings::load(&workspace).expect("加载测试配置失败");
        let store = Arc::new(SqliteStore::new(&settings.database_path).expect("创建数据库失败"));
        let manager = SessionManager::new(settings.clone(), store.clone());

        let session_dir = settings.sessions_root.join("session-demo");
        let mut db_session = Session::new(
            "session-demo".to_string(),
            "directory-demo".to_string(),
            "demo".to_string(),
            "demo-project".to_string(),
            workspace.to_string_lossy().to_string(),
            workspace.to_string_lossy().to_string(),
            "deepseek-v4-flash".to_string(),
            ApprovalMode::AskUser,
            true,
            session_dir.clone(),
            "system prompt".to_string(),
        );
        db_session.round = 5;
        db_session.last_round_no = 5;
        db_session.content_hash = "db_hash".to_string();
        manager
            .save_snapshot(&db_session, &[Message::user("from db")])
            .expect("保存数据库快照失败");

        let mut file_session = db_session.clone();
        file_session.round = 2;
        file_session.last_round_no = 2;
        file_session.content_hash = "file_hash".to_string();
        write_session_ini(&file_session).expect("写入旧的 session.ini 失败");
        write_memory_json(&session_dir, &[Message::user("from file")])
            .expect("写入旧的 memory.json 失败");

        let report = manager.repair_from_snapshots().expect("执行恢复失败");
        let repaired = read_session_ini(&session_dir).expect("读取修复后的 session.ini 失败");
        let messages = manager
            .load_messages(&repaired)
            .expect("读取修复后的消息失败");

        assert_eq!(report.restored_from_database, 1);
        assert_eq!(repaired.last_round_no, 5);
        assert_eq!(messages[0].content, "from db");
    }

    #[test]
    fn should_rebuild_database_from_file_when_hash_conflicts() {
        let workspace = PathBuf::from(format!(
            "target/test_repair_hash_conflict_{}",
            uuid::Uuid::new_v4()
        ));
        let settings = Settings::load(&workspace).expect("加载测试配置失败");
        let store = Arc::new(SqliteStore::new(&settings.database_path).expect("创建数据库失败"));
        let manager = SessionManager::new(settings.clone(), store.clone());

        let session_dir = settings.sessions_root.join("session-demo");
        let mut db_session = Session::new(
            "session-demo".to_string(),
            "directory-demo".to_string(),
            "demo".to_string(),
            "demo-project".to_string(),
            workspace.to_string_lossy().to_string(),
            workspace.to_string_lossy().to_string(),
            "deepseek-v4-flash".to_string(),
            ApprovalMode::AskUser,
            true,
            session_dir.clone(),
            "system prompt".to_string(),
        );
        db_session.round = 3;
        db_session.last_round_no = 3;
        db_session.content_hash = "db_hash".to_string();
        manager
            .save_snapshot(&db_session, &[Message::user("from db")])
            .expect("保存数据库快照失败");

        let mut file_session = db_session.clone();
        file_session.content_hash = "file_hash".to_string();
        write_session_ini(&file_session).expect("写入冲突 session.ini 失败");
        write_memory_json(&session_dir, &[Message::user("from file")])
            .expect("写入冲突 memory.json 失败");

        let report = manager.repair_from_snapshots().expect("执行恢复失败");
        let session = manager
            .use_session("session-demo")
            .expect("读取恢复后的会话失败");
        let messages = manager
            .load_messages(&session)
            .expect("读取恢复后的消息失败");

        assert_eq!(report.rebuilt_database_from_file, 1);
        assert_eq!(session.content_hash, "file_hash");
        assert_eq!(messages[0].content, "from file");
    }

    #[test]
    fn should_reject_deleting_running_session() {
        let workspace = PathBuf::from(format!(
            "target/test_delete_running_session_{}",
            uuid::Uuid::new_v4()
        ));
        let settings = Settings::load(&workspace).expect("加载测试配置失败");
        let store = Arc::new(SqliteStore::new(&settings.database_path).expect("创建数据库失败"));
        let manager = SessionManager::new(settings.clone(), store);

        let session_dir = settings.sessions_root.join("running-session");
        let mut session = Session::new(
            "running-session".to_string(),
            "directory-demo".to_string(),
            "demo".to_string(),
            "demo-project".to_string(),
            workspace.to_string_lossy().to_string(),
            workspace.to_string_lossy().to_string(),
            "deepseek-v4-flash".to_string(),
            ApprovalMode::AskUser,
            true,
            session_dir,
            "system prompt".to_string(),
        );
        session.status = crate::domain::SessionStatus::Running;
        manager
            .save_snapshot(&session, &[Message::user("hello")])
            .expect("保存运行中会话失败");

        let error = manager
            .delete_session("running-session", "test")
            .expect_err("运行中会话不应允许删除");
        assert!(error.to_string().contains("禁止删除"));
    }

    #[test]
    fn should_reject_deleting_workspace_with_running_session() {
        let workspace = PathBuf::from(format!(
            "target/test_delete_running_workspace_{}",
            uuid::Uuid::new_v4()
        ));
        let settings = Settings::load(&workspace).expect("加载测试配置失败");
        let store = Arc::new(SqliteStore::new(&settings.database_path).expect("创建数据库失败"));
        let manager = SessionManager::new(settings.clone(), store);

        let workspace_entity = manager.build_workspace_directory();
        manager
            .store
            .upsert_workspace(&workspace_entity)
            .expect("写入工作区失败");

        let session_dir = settings.sessions_root.join("running-session");
        let mut session = Session::new(
            "running-session".to_string(),
            workspace_entity.id.clone(),
            "demo".to_string(),
            workspace_entity.project_name.clone(),
            workspace_entity.project_path.clone(),
            workspace.to_string_lossy().to_string(),
            "deepseek-v4-flash".to_string(),
            ApprovalMode::AskUser,
            true,
            session_dir,
            "system prompt".to_string(),
        );
        session.status = crate::domain::SessionStatus::Running;
        manager
            .save_snapshot(&session, &[Message::user("hello")])
            .expect("保存运行中会话失败");

        let error = manager
            .delete_workspace(&workspace_entity.id, "test")
            .expect_err("存在运行中会话时不应允许删除工作区");
        assert!(error.to_string().contains("禁止删除工作区"));
    }
}
