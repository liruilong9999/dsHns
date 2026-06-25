use dshns_core::config::ApprovalMode;
use dshns_core::tool::ToolCall;
use regex::Regex;

pub struct Approver { mode: ApprovalMode, danger: Vec<Regex> }

pub enum ApprovalVerdict { Allow, NeedsConfirmation { reason: String } }

impl Approver {
    pub fn new(mode: ApprovalMode) -> Self {
        let danger = vec![
            r"(?i)\bremove-item\b", r"(?i)\bdel\b\s",
            r"C:\\Windows", r"C:\\Program Files",
            r"(?i)invoke-webrequest.*\|.*invoke-expression",
        ].into_iter().map(|p| Regex::new(p).unwrap()).collect();
        Self { mode, danger }
    }

    pub fn set_mode(&mut self, m: ApprovalMode) { self.mode = m; }
    pub fn mode(&self) -> ApprovalMode { self.mode }

    pub fn check(&self, call: &ToolCall) -> ApprovalVerdict {
        match self.mode {
            ApprovalMode::Auto => ApprovalVerdict::Allow,
            ApprovalMode::Paranoid => ApprovalVerdict::NeedsConfirmation { reason: format!("确认执行 {}？", call.name) },
            ApprovalMode::Confirm => {
                if call.name == "exec_shell" {
                    if let Some(cmd) = call.arguments["cmd"].as_str() {
                        for p in &self.danger {
                            if p.is_match(cmd) { return ApprovalVerdict::NeedsConfirmation { reason: format!("危险命令: {}", cmd) }; }
                        }
                    }
                }
                ApprovalVerdict::Allow
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; use serde_json::json;
    fn mk_call(name: &str, cmd: &str) -> ToolCall { ToolCall { id: "1".into(), name: name.into(), arguments: json!({"cmd": cmd}) } }
    #[test] fn test_auto() { assert!(matches!(Approver::new(ApprovalMode::Auto).check(&mk_call("exec_shell","rm x")), ApprovalVerdict::Allow)); }
    #[test] fn test_paranoid() { assert!(matches!(Approver::new(ApprovalMode::Paranoid).check(&mk_call("read_file","")), ApprovalVerdict::NeedsConfirmation{..})); }
    #[test] fn test_confirm_danger() { assert!(matches!(Approver::new(ApprovalMode::Confirm).check(&mk_call("exec_shell","Remove-Item x")), ApprovalVerdict::NeedsConfirmation{..})); }
}
