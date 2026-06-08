//! 工具执行器实现。

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::approval::manager::ApprovalManager;
use crate::domain::{
    AssistantToolCall, ToolCallRecord, ToolFailureType, ToolProjectionType, ToolResultRecord,
};
use crate::ipc::bus::EventBus;
use crate::persistence::sqlite::SqliteStore;
use crate::session::snapshot::TOOL_RESULT_INDEX_NAME;
use crate::skill::manager::SkillManager;
use crate::tools::registry::{ToolExecutionContext, ToolRegistry};
use crate::utils::fs::{read_optional_utf8, write_utf8};
use crate::utils::time::now_rfc3339;

/// 工具执行返回值。
pub struct ToolExecutionReceipt {
    /// 工具调用标识。
    pub tool_call_id: String,
    /// 工具名称。
    pub tool_name: String,
    /// 是否成功。
    pub success: bool,
    /// 输出正文。
    pub output: String,
    /// 错误信息。
    pub error_message: String,
    /// 投影内容。
    pub projection_content: String,
    /// 失败类型。
    pub failure_type: Option<ToolFailureType>,
}

/// 工具执行器。
pub struct ToolExecutor {
    /// 工具注册表。
    registry: Arc<ToolRegistry>,
    /// 审批管理器。
    approval_manager: ApprovalManager,
    /// 工作区根目录。
    workspace_root: std::path::PathBuf,
    /// Shell 程序名。
    shell_program: String,
    /// Skill 管理器。
    skill_manager: SkillManager,
    /// 工具结果内联阈值。
    inline_output_limit: usize,
    /// SQLite 存储。
    store: Arc<SqliteStore>,
    /// 事件总线。
    event_bus: Arc<EventBus>,
}

impl ToolExecutor {
    /// 创建工具执行器。
    pub fn new(
        registry: Arc<ToolRegistry>,
        approval_manager: ApprovalManager,
        workspace_root: std::path::PathBuf,
        shell_program: String,
        skill_manager: SkillManager,
        inline_output_limit: usize,
        store: Arc<SqliteStore>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            registry,
            approval_manager,
            workspace_root,
            shell_program,
            skill_manager,
            inline_output_limit,
            store,
            event_bus,
        }
    }

    /// 执行工具并完成结果持久化。
    pub async fn execute(
        &self,
        session_id: &str,
        round_no: i64,
        session_dir: &Path,
        tool_call: &AssistantToolCall,
    ) -> ToolExecutionReceipt {
        let tool_name = tool_call.function.name.clone();
        let parsed_args = match serde_json::from_str::<Value>(&tool_call.function.arguments) {
            Ok(value) => value,
            Err(error) => {
                let message = format!("工具参数不是合法 JSON：{}", error);
                let receipt = ToolExecutionReceipt {
                    tool_call_id: tool_call.id.clone(),
                    tool_name,
                    success: false,
                    output: String::new(),
                    error_message: message.clone(),
                    projection_content: message.clone(),
                    failure_type: Some(ToolFailureType::InvalidArgs),
                };
                let _ = self.persist(
                    session_id,
                    round_no,
                    session_dir,
                    &tool_call.function.arguments,
                    &receipt,
                );
                return receipt;
            }
        };

        let registered = match self.registry.get(&tool_name) {
            Ok(value) => value,
            Err(error) => {
                let message = error.to_string();
                let receipt = ToolExecutionReceipt {
                    tool_call_id: tool_call.id.clone(),
                    tool_name,
                    success: false,
                    output: String::new(),
                    error_message: message.clone(),
                    projection_content: message.clone(),
                    failure_type: Some(ToolFailureType::ExecError),
                };
                let _ = self.persist(
                    session_id,
                    round_no,
                    session_dir,
                    &tool_call.function.arguments,
                    &receipt,
                );
                return receipt;
            }
        };

        let decision = match self.approval_manager.approve_with_events(
            &self.event_bus,
            session_id,
            round_no,
            tool_call,
            &registered.definition,
            &parsed_args,
        ) {
            Ok(value) => value,
            Err(error) => {
                let message = error.to_string();
                let receipt = ToolExecutionReceipt {
                    tool_call_id: tool_call.id.clone(),
                    tool_name,
                    success: false,
                    output: String::new(),
                    error_message: message.clone(),
                    projection_content: message.clone(),
                    failure_type: Some(ToolFailureType::ApprovalDenied),
                };
                let _ = self.persist(
                    session_id,
                    round_no,
                    session_dir,
                    &tool_call.function.arguments,
                    &receipt,
                );
                return receipt;
            }
        };

        if !decision.approved {
            let receipt = ToolExecutionReceipt {
                tool_call_id: tool_call.id.clone(),
                tool_name,
                success: false,
                output: String::new(),
                error_message: decision.reason.clone(),
                projection_content: decision.reason.clone(),
                failure_type: Some(ToolFailureType::ApprovalDenied),
            };
            let _ = self.persist(
                session_id,
                round_no,
                session_dir,
                &tool_call.function.arguments,
                &receipt,
            );
            return receipt;
        }

        let context = ToolExecutionContext {
            workspace_root: self.workspace_root.clone(),
            session_dir: session_dir.to_path_buf(),
            shell_program: self.shell_program.clone(),
            skill_manager: self.skill_manager.clone(),
        };

        let receipt = match registered.handler.handle(parsed_args, &context).await {
            Ok(output) => ToolExecutionReceipt {
                tool_call_id: tool_call.id.clone(),
                tool_name,
                success: true,
                projection_content: self.build_projection(&output),
                output,
                error_message: String::new(),
                failure_type: None,
            },
            Err(error) => {
                let message = error.to_string();
                let failure_type = if message.contains("参数非法")
                    || message.contains("replace_range")
                    || message.contains("缺少")
                    || message.contains("冲突")
                {
                    ToolFailureType::InvalidArgs
                } else {
                    ToolFailureType::ExecError
                };
                ToolExecutionReceipt {
                    tool_call_id: tool_call.id.clone(),
                    tool_name,
                    success: false,
                    output: String::new(),
                    error_message: message.clone(),
                    projection_content: message,
                    failure_type: Some(failure_type),
                }
            }
        };

        let _ = self.persist(
            session_id,
            round_no,
            session_dir,
            &tool_call.function.arguments,
            &receipt,
        );
        receipt
    }

    /// 生成适合注入回模型的投影内容。
    fn build_projection(&self, output: &str) -> String {
        if output.chars().count() <= self.inline_output_limit {
            output.to_string()
        } else {
            let preview: String = output.chars().take(self.inline_output_limit).collect();
            format!(
                "{}\n\n输出过长，完整结果已外置保存，请根据句柄按需读取。",
                preview
            )
        }
    }

    /// 持久化工具结果索引与正文。
    fn persist(
        &self,
        session_id: &str,
        round_no: i64,
        session_dir: &Path,
        arguments_json: &str,
        receipt: &ToolExecutionReceipt,
    ) -> Result<()> {
        let handle = format!("tool:{}", receipt.tool_call_id);
        let effective_text = if receipt.success {
            &receipt.output
        } else {
            &receipt.error_message
        };
        let char_count = effective_text.chars().count();
        let byte_count = effective_text.len();
        let externalized = char_count > self.inline_output_limit;
        let body_file_path = if externalized {
            let path = session_dir
                .join("tool_results")
                .join(format!("{}.txt", receipt.tool_call_id));
            write_utf8(&path, effective_text)?;
            path.to_string_lossy().to_string()
        } else {
            String::new()
        };
        let preview_head: String = effective_text.chars().take(120).collect();
        let preview_tail = {
            let chars: Vec<char> = effective_text.chars().collect();
            let start = chars.len().saturating_sub(120);
            chars[start..].iter().collect::<String>()
        };

        let projection_type = if externalized {
            if char_count > self.inline_output_limit * 4 {
                ToolProjectionType::SummaryOnly
            } else {
                ToolProjectionType::SummaryWithPreview
            }
        } else {
            ToolProjectionType::InlineFull
        };
        let projection_content = match projection_type {
            ToolProjectionType::InlineFull => receipt.projection_content.clone(),
            ToolProjectionType::SummaryWithPreview => format!(
                "{}\n\npreview_head:\n{}\n\npreview_tail:\n{}",
                receipt.projection_content, preview_head, preview_tail
            ),
            ToolProjectionType::SummaryOnly => receipt
                .projection_content
                .lines()
                .next()
                .unwrap_or_default()
                .to_string(),
        };

        let record = ToolResultRecord {
            tool_call_id: receipt.tool_call_id.clone(),
            tool_name: receipt.tool_name.clone(),
            handle,
            body_file_path,
            projection_type,
            projection_content,
            summary: if receipt.success {
                format!("工具 {} 执行成功", receipt.tool_name)
            } else {
                format!(
                    "工具 {} 执行失败：{}",
                    receipt.tool_name, receipt.error_message
                )
            },
            preview_head,
            preview_tail,
            char_count,
            byte_count,
            success: receipt.success,
            truncated: externalized,
            externalized,
            updated_at: now_rfc3339(),
        };

        let index_path = session_dir.join(TOOL_RESULT_INDEX_NAME);
        let original = read_optional_utf8(&index_path)?.unwrap_or_else(|| "[]".to_string());
        let mut records: Vec<ToolResultRecord> =
            serde_json::from_str(&original).unwrap_or_default();
        records.push(record.clone());
        let json = serde_json::to_string_pretty(&records)?;
        write_utf8(&index_path, &json)?;

        let tool_call_record = ToolCallRecord {
            id: receipt.tool_call_id.clone(),
            session_id: session_id.to_string(),
            round_no,
            tool_name: receipt.tool_name.clone(),
            arguments_json: arguments_json.to_string(),
            status: if receipt.success {
                "done".to_string()
            } else {
                "failed".to_string()
            },
            success: receipt.success,
            failure_type: receipt.failure_type.clone(),
            error_message: receipt.error_message.clone(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };

        self.store.insert_tool_call(&tool_call_record)?;
        self.store.insert_tool_result_index(&record)?;
        Ok(())
    }
}
