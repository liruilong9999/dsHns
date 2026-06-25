use std::sync::Arc;
use std::time::Duration;
use dshns_core::tool::{ToolCall, ToolOutcome, ToolStatus};
use crate::registry::ToolRegistry;

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    default_timeout: Duration,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, timeout_secs: u64) -> Self {
        Self { registry, default_timeout: Duration::from_secs(timeout_secs) }
    }

    pub async fn exec_one(&self, call: &ToolCall) -> ToolOutcome {
        let tool = match self.registry.get(&call.name) {
            Some(t) => t,
            None => return ToolOutcome {
                call_id: call.id.clone(),
                status: ToolStatus::Error { reason: format!("未知工具: {}", call.name) },
                content: String::new(), was_truncated: false,
            },
        };
        match tokio::time::timeout(self.default_timeout, tool.execute(call)).await {
            Ok(outcome) => outcome,
            Err(_) => ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Timeout, content: format!("工具 {} 执行超时", call.name), was_truncated: false },
        }
    }

    pub async fn exec_many(&self, calls: &[ToolCall]) -> Vec<ToolOutcome> {
        futures::future::join_all(calls.iter().map(|c| self.exec_one(c))).await
    }
}
