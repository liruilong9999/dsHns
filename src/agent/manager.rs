//! 子 Agent 生命周期管理器。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{AgentInstance, AgentMode, AgentStatus};
use crate::persistence::sqlite::SqliteStore;
use crate::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use crate::utils::hash::sha256_hex;
use crate::utils::time::now_rfc3339;

const DEFAULT_MAX_LEVEL: i32 = 2;
const DEFAULT_MAX_TOTAL: usize = 5;

/// 子 Agent 管理器。
pub struct SubagentManager {
    /// 父会话目录。
    session_dir: PathBuf,
}

impl SubagentManager {
    /// 创建子 Agent 管理器。
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    /// 创建子 Agent。
    pub fn open(
        &self,
        mode: &str,
        inherit_context: bool,
        allowed_paths: Vec<String>,
        task_spec: Value,
        parent_agent_id: Option<String>,
    ) -> Result<AgentInstance> {
        let mut agents = self.load_agents()?;
        if agents.len() >= DEFAULT_MAX_TOTAL {
            return Err(anyhow!(
                "子 Agent 总数超过上限 {}，当前无法继续创建",
                DEFAULT_MAX_TOTAL
            ));
        }

        let level = if let Some(parent_id) = parent_agent_id.as_ref() {
            let parent = agents
                .iter()
                .find(|agent| agent.id == *parent_id)
                .ok_or_else(|| anyhow!("未找到父 Agent：{}", parent_id))?;
            parent.level + 1
        } else {
            1
        };

        if level > DEFAULT_MAX_LEVEL {
            return Err(anyhow!(
                "子 Agent 层级超过上限 {}，当前无法继续创建",
                DEFAULT_MAX_LEVEL
            ));
        }

        let id = Uuid::new_v4().to_string();
        let agent_dir = self.session_dir.join(".tools").join("agent").join(&id);
        ensure_directory(&agent_dir)?;

        let mode_enum = match mode {
            "isolate" | "Isolate" => AgentMode::Isolate,
            _ => AgentMode::Inherit,
        };
        let task_spec_json = serde_json::to_string(&task_spec)?;
        let allowed_paths_json = serde_json::to_string(&allowed_paths)?;
        let now = now_rfc3339();
        let parent_session_id = self
            .session_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown-session")
            .to_string();

        let agent = AgentInstance {
            id: id.clone(),
            parent_session_id,
            parent_agent_id,
            mode: mode_enum,
            inherit_context,
            level,
            status: AgentStatus::Open,
            session_dir: agent_dir.to_string_lossy().to_string(),
            child_session_id: Uuid::new_v4().to_string(),
            allowed_paths_json,
            task_spec_json: task_spec_json.clone(),
            constraint_hash: sha256_hex(&format!(
                "{}:{}:{}",
                inherit_context,
                task_spec_json,
                serde_json::to_string(&allowed_paths)?
            )),
            result_summary: String::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        write_utf8(
            &agent_dir.join("task_spec.json"),
            &serde_json::to_string_pretty(&task_spec)?,
        )?;
        agents.push(agent.clone());
        self.save_agents(&agents)?;
        self.persist_agent_instance(&agent)?;
        Ok(agent)
    }

    /// 向子 Agent 派发执行请求。
    pub fn eval(&self, agent_id: &str, input: Value) -> Result<Value> {
        let mut agents = self.load_agents()?;
        let index = agents
            .iter()
            .position(|agent| agent.id == agent_id)
            .ok_or_else(|| anyhow!("未找到子 Agent：{}", agent_id))?;

        agents[index].status = AgentStatus::Running;
        agents[index].updated_at = now_rfc3339();
        self.save_agents(&agents)?;
        self.persist_agent_instance(&agents[index])?;

        let agent_dir = Path::new(&agents[index].session_dir);
        write_utf8(
            &agent_dir.join("eval_input.json"),
            &serde_json::to_string_pretty(&input)?,
        )?;

        let summary = format!(
            "已记录子 Agent 任务执行请求，Agent={}，输入摘要={}",
            agent_id,
            render_summary(&input)
        );
        agents[index].status = AgentStatus::Done;
        agents[index].result_summary = summary.clone();
        agents[index].updated_at = now_rfc3339();
        self.save_agents(&agents)?;
        self.persist_agent_instance(&agents[index])?;

        Ok(serde_json::json!({
            "agent_id": agents[index].id,
            "status": "done",
            "child_session_id": agents[index].child_session_id,
            "result_summary": summary
        }))
    }

    /// 关闭子 Agent。
    pub fn close(&self, agent_id: &str) -> Result<AgentInstance> {
        let mut agents = self.load_agents()?;
        let index = agents
            .iter()
            .position(|agent| agent.id == agent_id)
            .ok_or_else(|| anyhow!("未找到子 Agent：{}", agent_id))?;
        agents[index].status = AgentStatus::Closed;
        agents[index].updated_at = now_rfc3339();
        let agent = agents[index].clone();
        self.save_agents(&agents)?;
        self.persist_agent_instance(&agent)?;
        Ok(agent)
    }

    fn load_agents(&self) -> Result<Vec<AgentInstance>> {
        let path = self.agents_file_path();
        let content = read_optional_utf8(&path)?.unwrap_or_else(|| "[]".to_string());
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    fn save_agents(&self, agents: &[AgentInstance]) -> Result<()> {
        write_utf8(
            &self.agents_file_path(),
            &serde_json::to_string_pretty(agents)?,
        )
    }

    fn agents_file_path(&self) -> PathBuf {
        self.session_dir
            .join(".tools")
            .join("agent")
            .join("agents.json")
    }

    fn persist_agent_instance(&self, agent: &AgentInstance) -> Result<()> {
        if let Some(store) = self.try_open_store()? {
            store.upsert_agent_instance(agent)?;
        }
        Ok(())
    }

    fn try_open_store(&self) -> Result<Option<SqliteStore>> {
        for ancestor in self.session_dir.ancestors() {
            let candidate = ancestor.join("harness.db");
            if candidate.exists() {
                return SqliteStore::new(&candidate).map(Some);
            }
        }
        Ok(None)
    }
}

fn render_summary(input: &Value) -> String {
    match input {
        Value::String(text) => truncate(text, 80),
        _ => truncate(&input.to_string(), 80),
    }
}

fn truncate(content: &str, limit: usize) -> String {
    let text: String = content.chars().take(limit).collect();
    if content.chars().count() > limit {
        format!("{}...", text)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::SubagentManager;

    #[test]
    fn should_open_and_close_subagent() {
        let session_dir = PathBuf::from(format!(
            "target/test_subagent_session_{}",
            uuid::Uuid::new_v4()
        ));
        let manager = SubagentManager::new(session_dir);
        let agent = manager
            .open(
                "isolate",
                false,
                vec!["src".to_string()],
                json!({"task":"demo"}),
                None,
            )
            .expect("创建子 Agent 失败");
        let closed = manager.close(&agent.id).expect("关闭子 Agent 失败");
        assert_eq!(closed.id, agent.id);
    }
}
