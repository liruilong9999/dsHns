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

/// 审批能力开关。
#[derive(Debug, Clone, Copy)]
pub struct ApprovalCapabilities {
    /// 是否允许网络工具。
    pub allow_network: bool,
    /// 是否允许 Shell 工具。
    pub allow_shell: bool,
    /// 是否允许写文件工具。
    pub allow_file_write: bool,
    /// 是否允许插件类工具。
    pub allow_plugin_tool: bool,
}

impl ApprovalCapabilities {
    /// 返回默认能力配置。
    pub fn new(
        allow_network: bool,
        allow_shell: bool,
        allow_file_write: bool,
        allow_plugin_tool: bool,
    ) -> Self {
        Self {
            allow_network,
            allow_shell,
            allow_file_write,
            allow_plugin_tool,
        }
    }
}

/// 审批管理器。
pub struct ApprovalManager {
    /// 当前审批模式。
    mode: ApprovalMode,
    /// 当前能力开关。
    capabilities: ApprovalCapabilities,
}

impl ApprovalManager {
    /// 创建审批管理器。
    pub fn new(mode: ApprovalMode, capabilities: ApprovalCapabilities) -> Self {
        Self { mode, capabilities }
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
        if let Some(reason) = self.blocked_by_capability(definition) {
            return Ok(ApprovalDecision {
                approved: false,
                reason,
            });
        }

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

    /// 判断能力开关是否直接阻断工具。
    fn blocked_by_capability(&self, definition: &ToolDefinition) -> Option<String> {
        match definition.risk_level {
            ToolRiskLevel::Network if !self.capabilities.allow_network => Some(format!(
                "当前配置已关闭网络能力，工具 {} 暂不可用。",
                definition.name
            )),
            ToolRiskLevel::Execute if !self.capabilities.allow_shell => Some(format!(
                "当前配置已关闭 Shell 执行能力，工具 {} 暂不可用。",
                definition.name
            )),
            ToolRiskLevel::Write if !self.capabilities.allow_file_write => Some(format!(
                "当前配置已关闭文件写入能力，工具 {} 暂不可用。",
                definition.name
            )),
            ToolRiskLevel::Agent if !self.capabilities.allow_plugin_tool => Some(format!(
                "当前配置已关闭扩展代理能力，工具 {} 暂不可用。",
                definition.name
            )),
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::domain::{ApprovalMode, ToolRiskLevel};
    use crate::tools::registry::ToolDefinition;

    use super::{ApprovalCapabilities, ApprovalManager};

    fn build_definition(name: &str, risk_level: ToolRiskLevel) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "测试工具".to_string(),
            parameters_schema: json!({ "type": "object" }),
            risk_level,
            visible_to_model: true,
        }
    }

    /// 验证能力开关关闭时会直接阻断写文件工具。
    #[test]
    fn should_block_write_tool_when_file_write_disabled() {
        let manager = ApprovalManager::new(
            ApprovalMode::FullAccess,
            ApprovalCapabilities::new(true, true, false, true),
        );
        let decision = manager
            .approve(
                &build_definition("write_file", ToolRiskLevel::Write),
                &json!({ "path": "a.txt" }),
            )
            .expect("审批失败");

        assert!(!decision.approved);
        assert!(decision.reason.contains("文件写入能力"));
    }

    /// 验证能力开关关闭时会阻断网络工具，返回中文不可用说明。
    #[test]
    fn should_block_network_tool_when_network_disabled() {
        let manager = ApprovalManager::new(
            ApprovalMode::FullAccess,
            ApprovalCapabilities::new(false, true, true, true),
        );
        let decision = manager
            .approve(
                &build_definition("web_search", ToolRiskLevel::Network),
                &json!({ "query": "rust" }),
            )
            .expect("审批失败");

        assert!(!decision.approved);
        assert!(decision.reason.contains("关闭网络能力"));
    }

    /// 验证 AutoApproveSafe 模式仍会自动放行低风险只读工具。
    #[test]
    fn should_auto_approve_readonly_tool_in_safe_mode() {
        let manager = ApprovalManager::new(
            ApprovalMode::AutoApproveSafe,
            ApprovalCapabilities::new(true, false, false, false),
        );
        let decision = manager
            .approve(
                &build_definition("read_file", ToolRiskLevel::ReadOnly),
                &json!({ "path": "a.txt" }),
            )
            .expect("审批失败");

        assert!(decision.approved);
        assert!(decision.reason.contains("自动放行"));
    }
}
