//! 应用主控层实现。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::agent::loop_runner::{AgentLoopRunner, TurnOutcome};
use crate::approval::manager::ApprovalManager;
use crate::config::settings::Settings;
use crate::domain::{ApprovalMode, DeletionAudit, Session, SessionStatus, WorkspaceDirectory};
use crate::ipc::bus::EventBus;
use crate::ipc::events::IpcEvent;
use crate::llm::client::LlmClient;
use crate::persistence::sqlite::SqliteStore;
use crate::prompt::assembler::PromptAssembler;
use crate::session::manager::SessionManager;
use crate::skill::manager::SkillManager;
use crate::tools::executor::ToolExecutor;
use crate::tools::registry::ToolRegistry;
use crate::utils::fs::read_optional_utf8;
use crate::utils::hash::sha256_hex;
use crate::utils::time::now_rfc3339;

/// 应用主控器。
pub struct Harness {
    /// 运行配置。
    settings: Settings,
    /// Session 管理器。
    session_manager: Arc<SessionManager>,
    /// 提示词组装器。
    prompt_assembler: PromptAssembler,
    /// Skill 管理器。
    skill_manager: SkillManager,
    /// 工具注册表。
    tool_registry: Arc<ToolRegistry>,
    /// 模型客户端。
    llm_client: Arc<LlmClient>,
    /// 当前选中的会话。
    current_session: Option<Session>,
}

impl Harness {
    /// 创建主控器。
    pub fn new(workspace_root: PathBuf) -> Result<Self> {
        let settings = Settings::load(&workspace_root)?;
        let skill_manager = SkillManager::new(settings.skill_roots.clone());
        let prompt_assembler = PromptAssembler::new(workspace_root, skill_manager.clone());
        let store = Arc::new(SqliteStore::new(&settings.database_path)?);
        let session_manager = Arc::new(SessionManager::new(settings.clone(), store));
        let recovery = session_manager.repair_from_snapshots()?;
        if recovery.restored_from_file > 0
            || recovery.restored_from_database > 0
            || recovery.rebuilt_database_from_file > 0
            || recovery.unresolved_conflicts > 0
        {
            tracing::info!(
                "启动恢复完成：文件恢复 {}，数据库回写 {}，文件重建数据库 {}，未解决冲突 {}",
                recovery.restored_from_file,
                recovery.restored_from_database,
                recovery.rebuilt_database_from_file,
                recovery.unresolved_conflicts
            );
        }
        let tool_registry = Arc::new(ToolRegistry::with_defaults());
        let llm_client = Arc::new(LlmClient::new(settings.deepseek_base_url.clone()));

        Ok(Self {
            settings,
            session_manager,
            prompt_assembler,
            skill_manager,
            tool_registry,
            llm_client,
            current_session: None,
        })
    }

    /// 创建并选中新会话。
    pub fn create_session(&mut self, name: &str) -> Result<&Session> {
        let prompt = self.prompt_assembler.assemble()?;
        let session = self.session_manager.create_session(
            name,
            &self.settings.default_model,
            self.settings.default_approval_mode,
            self.settings.default_stream_output,
            prompt,
        )?;
        let bus = EventBus::new(session.session_dir.clone());
        bus.emit_session_status(&session.id, session.round, session.status.as_str())?;
        self.current_session = Some(session);
        self.current_session
            .as_ref()
            .ok_or_else(|| anyhow!("创建会话后未能保存当前会话"))
    }

    /// 切换当前会话。
    pub fn use_session(&mut self, key: &str) -> Result<&Session> {
        let session = self.session_manager.use_session(key)?;
        self.current_session = Some(session);
        self.current_session
            .as_ref()
            .ok_or_else(|| anyhow!("切换会话后未能保存当前会话"))
    }

    /// 列出全部会话。
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        self.session_manager.list_sessions()
    }

    /// 列出工作区元数据。
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceDirectory>> {
        self.session_manager.list_workspaces()
    }

    /// 列出删除审计记录。
    pub fn list_deletion_audits(&self, target_type: Option<&str>) -> Result<Vec<DeletionAudit>> {
        self.session_manager.list_deletion_audits(target_type)
    }

    /// 列出当前会话事件。
    pub fn list_current_events(&self) -> Result<Vec<IpcEvent>> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法查看事件"))?;
        EventBus::new(session.session_dir.clone()).list_events()
    }

    /// 读取当前会话最近一次 Token 统计。
    pub fn latest_current_token_usage(
        &self,
    ) -> Result<Option<crate::ipc::bus::TokenUsageSnapshot>> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法查看 Token 统计"))?;
        EventBus::new(session.session_dir.clone()).latest_token_usage()
    }

    /// 列出当前会话的工作记忆。
    pub fn list_current_working_memories(&self) -> Result<Vec<crate::domain::WorkingMemoryEntry>> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法查看工作记忆"))?;
        self.session_manager.list_working_memories(&session.id)
    }

    /// 列出当前会话的子 Agent。
    pub fn list_current_agents(&self) -> Result<Vec<crate::domain::AgentInstance>> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法查看子 Agent"))?;
        self.session_manager.list_agent_instances(&session.id)
    }

    /// 列出当前会话的工具调用记录。
    pub fn list_current_tool_calls(&self) -> Result<Vec<crate::domain::ToolCallRecord>> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法查看工具调用"))?;
        self.session_manager.list_tool_calls(&session.id)
    }

    /// 列出当前会话的工具结果索引。
    pub fn list_current_tool_results(&self) -> Result<Vec<crate::domain::ToolResultRecord>> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法查看工具结果"))?;
        self.session_manager.list_tool_result_indexes(&session.id)
    }

    /// 读取启动恢复日志。
    pub fn read_recovery_log(&self) -> Result<String> {
        let path = self.settings.data_root.join("recovery.log");
        read_optional_utf8(&path)?.ok_or_else(|| anyhow!("恢复日志不存在：{}", path.display()))
    }

    /// 读取当前会话下的工具结果句柄。
    pub fn read_tool_result_handle(&self, handle: &str) -> Result<String> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法读取工具结果"))?;
        self.session_manager
            .read_tool_result_by_handle(&session.id, handle)
    }

    /// 按工具调用标识读取当前会话的工具结果正文。
    pub fn read_tool_result_body(&self, tool_call_id: &str) -> Result<String> {
        let session = self
            .current_session
            .as_ref()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法读取工具结果"))?;
        self.session_manager
            .read_tool_result_by_call_id(&session.id, tool_call_id)
    }

    /// 获取当前会话。
    pub fn current_session(&self) -> Option<&Session> {
        self.current_session.as_ref()
    }

    /// 获取配置引用。
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// 运行普通用户输入。
    pub async fn run_user_input(&mut self, user_input: &str) -> Result<TurnOutcome> {
        let mut session = self
            .current_session
            .clone()
            .ok_or_else(|| anyhow!("当前尚未选择会话，请先执行 /create 或 /session use"))?;
        let event_bus = Arc::new(EventBus::new(session.session_dir.clone()));

        session.status = SessionStatus::Running;
        session.updated_at = now_rfc3339();
        event_bus.emit_session_status(&session.id, session.round + 1, session.status.as_str())?;

        let mut messages = self.session_manager.load_messages(&session)?;
        let approval_manager = ApprovalManager::new(session.approval_mode);
        let tool_executor = Arc::new(ToolExecutor::new(
            self.tool_registry.clone(),
            approval_manager,
            self.settings.workspace_root.clone(),
            self.settings.shell_program.clone(),
            self.skill_manager.clone(),
            self.settings.inline_output_limit,
            self.session_manager.store(),
            event_bus.clone(),
        ));
        let runner = AgentLoopRunner::new(
            self.llm_client.clone(),
            tool_executor,
            self.tool_registry.clone(),
            self.session_manager.clone(),
            self.settings.clone(),
            event_bus.clone(),
        );

        let outcome = runner.run_turn(&session, &mut messages, user_input).await?;
        session.round += 1;
        session.status = SessionStatus::Finished;
        session.last_round_no = session.round;
        session.snapshot_version += 1;
        session.content_hash = sha256_hex(&serde_json::to_string(&messages)?);
        session.updated_at = now_rfc3339();
        self.session_manager.save_snapshot(&session, &messages)?;
        event_bus.emit_session_status(&session.id, session.round, session.status.as_str())?;
        self.current_session = Some(session);
        Ok(outcome)
    }

    /// 取消当前轮执行，并将状态标记为已取消。
    pub fn cancel_current_turn(&mut self) -> Result<()> {
        let mut session = self
            .current_session
            .clone()
            .ok_or_else(|| anyhow!("当前尚未选择会话，无法取消执行"))?;
        if session.status != SessionStatus::Running {
            return Ok(());
        }

        session.status = SessionStatus::Cancelled;
        session.updated_at = now_rfc3339();
        let messages = self.session_manager.load_messages(&session)?;
        self.session_manager.save_snapshot(&session, &messages)?;
        let event_bus = EventBus::new(session.session_dir.clone());
        event_bus.emit_session_status(&session.id, session.round + 1, session.status.as_str())?;
        self.current_session = Some(session);
        Ok(())
    }

    /// 删除会话。
    pub fn delete_session(&mut self, key: &str) -> Result<String> {
        let audit_id = self.session_manager.delete_session(key, "cli")?;
        if self
            .current_session
            .as_ref()
            .map(|session| session.id == key || session.name == key)
            .unwrap_or(false)
        {
            self.current_session = None;
        }
        Ok(audit_id)
    }

    /// 恢复会话并切换为当前会话。
    pub fn restore_session(&mut self, key: &str) -> Result<&Session> {
        let session = self.session_manager.restore_session(key)?;
        self.current_session = Some(session);
        self.current_session
            .as_ref()
            .ok_or_else(|| anyhow!("恢复会话后未能刷新当前会话"))
    }

    /// 删除工作区及其会话元数据。
    pub fn delete_workspace(&mut self, key: &str) -> Result<String> {
        let audit_id = self.session_manager.delete_workspace(key, "cli")?;
        self.current_session = None;
        Ok(audit_id)
    }

    /// 恢复工作区及其会话。
    pub fn restore_workspace(&self, key: &str) -> Result<WorkspaceDirectory> {
        self.session_manager.restore_workspace(key)
    }

    /// 校验模型是否可用。
    pub fn check_model(&self, model: &str) -> Result<()> {
        if self.settings.is_allowed_model(model) {
            Ok(())
        } else {
            Err(anyhow!("当前模型不在允许清单中：{}", model))
        }
    }

    /// 返回可用模型列表。
    pub fn models(&self) -> &[String] {
        &self.settings.allowed_models
    }

    /// 返回当前审批模式。
    pub fn current_mode(&self) -> ApprovalMode {
        self.current_session
            .as_ref()
            .map(|session| session.approval_mode)
            .unwrap_or(self.settings.default_approval_mode)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::domain::SessionStatus;

    use super::Harness;

    #[test]
    fn should_mark_running_session_cancelled() {
        let workspace = PathBuf::from(format!(
            "target/test_cancel_workspace_{}",
            uuid::Uuid::new_v4()
        ));
        let mut harness = Harness::new(workspace).expect("创建主控器失败");
        harness.create_session("demo").expect("创建会话失败");
        harness
            .current_session
            .as_mut()
            .expect("当前会话不存在")
            .status = SessionStatus::Running;
        harness.cancel_current_turn().expect("取消当前轮失败");
        assert_eq!(
            harness.current_session().expect("当前会话不存在").status,
            SessionStatus::Cancelled
        );
    }
}
