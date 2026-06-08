//! 事件总线实现。

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;
use uuid::Uuid;

use crate::ipc::events::{EventType, IpcEvent};
use crate::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use crate::utils::time::now_rfc3339;

/// 事件总线。
#[derive(Clone)]
pub struct EventBus {
    /// 会话目录。
    session_dir: PathBuf,
}

impl EventBus {
    /// 创建事件总线。
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    /// 初始化事件文件。
    pub fn ensure_store(&self) -> Result<()> {
        let path = self.events_file_path();
        if let Some(parent) = path.parent() {
            ensure_directory(parent)?;
        }
        if !path.exists() {
            write_utf8(&path, "[]\n")?;
        }
        Ok(())
    }

    /// 记录会话状态事件。
    pub fn emit_session_status(&self, session_id: &str, round_no: i64, status: &str) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::SessionStatusChanged,
            payload: json!({ "status": status }),
        })
    }

    /// 记录工具状态事件。
    pub fn emit_tool_status(
        &self,
        session_id: &str,
        round_no: i64,
        tool_call_id: &str,
        tool_name: &str,
        status: &str,
        success: Option<bool>,
    ) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::ToolStatusChanged,
            payload: json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "status": status,
                "success": success
            }),
        })
    }

    /// 记录 Token 用量事件。
    pub fn emit_token_usage(
        &self,
        session_id: &str,
        round_no: i64,
        input_tokens: usize,
        output_tokens: usize,
        remaining_context: usize,
    ) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::TokenUsageUpdated,
            payload: json!({
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "remaining_context": remaining_context
            }),
        })
    }

    /// 记录工作记忆事件。
    pub fn emit_working_memory(
        &self,
        session_id: &str,
        round_no: i64,
        content: &str,
    ) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::WorkingMemoryCreated,
            payload: json!({
                "content_preview": truncate(content, 200)
            }),
        })
    }

    /// 记录审批请求事件。
    pub fn emit_approval_requested(
        &self,
        session_id: &str,
        round_no: i64,
        tool_name: &str,
        tool_call_id: &str,
        mode: &str,
    ) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::ApprovalRequested,
            payload: json!({
                "tool_name": tool_name,
                "tool_call_id": tool_call_id,
                "approval_mode": mode
            }),
        })
    }

    /// 记录审批完成事件。
    pub fn emit_approval_resolved(
        &self,
        session_id: &str,
        round_no: i64,
        tool_name: &str,
        tool_call_id: &str,
        approved: bool,
        reason: &str,
    ) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::ApprovalResolved,
            payload: json!({
                "tool_name": tool_name,
                "tool_call_id": tool_call_id,
                "approved": approved,
                "reason": reason
            }),
        })
    }

    /// 记录流式文本块事件。
    pub fn emit_stream_chunk(&self, session_id: &str, round_no: i64, chunk: &str) -> Result<()> {
        self.emit(IpcEvent {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            timestamp: now_rfc3339(),
            round_no,
            event_type: EventType::StreamChunkReceived,
            payload: json!({
                "chunk": truncate(chunk, 500)
            }),
        })
    }

    /// 读取当前事件列表。
    pub fn list_events(&self) -> Result<Vec<IpcEvent>> {
        let content =
            read_optional_utf8(&self.events_file_path())?.unwrap_or_else(|| "[]".to_string());
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    /// 写入通用事件。
    fn emit(&self, event: IpcEvent) -> Result<()> {
        self.ensure_store()?;
        let path = self.events_file_path();
        let content = read_optional_utf8(&path)?.unwrap_or_else(|| "[]".to_string());
        let mut events: Vec<IpcEvent> = serde_json::from_str(&content).unwrap_or_default();
        events.push(event);
        write_utf8(&path, &serde_json::to_string_pretty(&events)?)
    }

    /// 事件持久化文件路径。
    fn events_file_path(&self) -> PathBuf {
        self.session_dir
            .join(".tools")
            .join("events")
            .join("events.json")
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

    use super::EventBus;

    #[test]
    fn should_persist_event() {
        let bus = EventBus::new(PathBuf::from("target/test_event_bus_session"));
        bus.emit_session_status("session-1", 1, "running")
            .expect("写入事件失败");
        let events = bus.list_events().expect("读取事件失败");
        assert!(!events.is_empty());
    }
}
