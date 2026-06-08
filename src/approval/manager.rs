//! 审批管理器实现。

use std::io::{self, Write};

use anyhow::Result;
use serde_json::Value;

use crate::domain::AssistantToolCall;
use crate::domain::{ApprovalMode, ToolRiskLevel};
use crate::ipc::bus::EventBus;
use crate::tools::registry::ToolDefinition;

/// 审批结果。
pub struct ApprovalDecision {
    /// 是否允许执行。
    pub approved: bool,
    /// 审批说明。
    pub reason: String,
}

/// 审批管理器。
pub struct ApprovalManager {
    /// 当前审批模式。
    mode: ApprovalMode,
}

impl ApprovalManager {
    /// 创建审批管理器。
    pub fn new(mode: ApprovalMode) -> Self {
        Self { mode }
    }

    /// 返回当前审批模式。
    pub fn mode(&self) -> ApprovalMode {
        self.mode
    }

    /// 对工具执行请求执行审批判断。
    pub fn approve(
        &self,
        definition: &ToolDefinition,
        arguments: &Value,
    ) -> Result<ApprovalDecision> {
        match self.mode {
            ApprovalMode::FullAccess => Ok(ApprovalDecision {
                approved: true,
                reason: "当前为 FullAccess 模式，已自动放行。".to_string(),
            }),
            ApprovalMode::AutoApproveSafe => {
                if definition.risk_level == ToolRiskLevel::ReadOnly {
                    Ok(ApprovalDecision {
                        approved: true,
                        reason: "当前为 AutoApproveSafe 模式，低风险只读工具已自动放行。"
                            .to_string(),
                    })
                } else {
                    Ok(ApprovalDecision {
                        approved: false,
                        reason: format!(
                            "当前为 AutoApproveSafe 模式，工具 {} 属于高风险操作，已阻止执行。",
                            definition.name
                        ),
                    })
                }
            }
            ApprovalMode::AskUser => self.ask_user(definition, arguments),
        }
    }

    /// 在审批前后写入事件并执行审批。
    pub fn approve_with_events(
        &self,
        event_bus: &EventBus,
        session_id: &str,
        round_no: i64,
        tool_call: &AssistantToolCall,
        definition: &ToolDefinition,
        arguments: &Value,
    ) -> Result<ApprovalDecision> {
        event_bus.emit_approval_requested(
            session_id,
            round_no,
            &definition.name,
            &tool_call.id,
            self.mode.as_str(),
        )?;
        let decision = self.approve(definition, arguments)?;
        event_bus.emit_approval_resolved(
            session_id,
            round_no,
            &definition.name,
            &tool_call.id,
            decision.approved,
            &decision.reason,
        )?;
        Ok(decision)
    }

    /// 通过命令行交互式向用户申请审批。
    fn ask_user(&self, definition: &ToolDefinition, arguments: &Value) -> Result<ApprovalDecision> {
        println!(
            "需要审批：工具 `{}`\n参数：{}\n请输入 y 允许，其它任意输入视为拒绝：",
            definition.name, arguments
        );
        print!("> ");
        io::stdout().flush()?;

        let mut buffer = String::new();
        io::stdin().read_line(&mut buffer)?;
        let approved = matches!(buffer.trim().to_ascii_lowercase().as_str(), "y" | "yes");

        Ok(ApprovalDecision {
            approved,
            reason: if approved {
                "用户已明确允许执行。".to_string()
            } else {
                "用户拒绝执行该工具。".to_string()
            },
        })
    }
}
