use dshns_core::tool::{ToolCall, ToolStatus, ToolOutcome};
use regex::Regex;

pub struct SafetyGuard { patterns: Vec<Regex> }

impl SafetyGuard {
    pub fn new() -> Self {
        let ps = vec![
            r"(?i)remove-item.*-recurse.*-force",
            r"(?i)\bdel\b.*/f.*/s",
            r"(?i)\brd\b.*/s.*/q",
            r"(?i)\brunas\b",
            r"(?i)^\s*format\s",
            r"(?i)\bdiskpart\b",
        ];
        Self { patterns: ps.into_iter().map(|p| Regex::new(p).unwrap()).collect() }
    }

    pub fn check(&self, call: &ToolCall) -> Option<ToolOutcome> {
        if call.name != "exec_shell" { return None; }
        let cmd = call.arguments["cmd"].as_str()?;
        for p in &self.patterns {
            if p.is_match(cmd) {
                let reason = if p.as_str().contains("remove-item") {
                    "递归强制删除已被系统禁止。请逐个文件使用 Remove-Item <路径> 删除。"
                } else if p.as_str().contains(r"\bdel\b") || p.as_str().contains(r"\brd\b") {
                    "递归强制删除已被系统禁止。请逐个指定文件路径。"
                } else if p.as_str().contains("runas") {
                    "提权操作已被系统禁止。"
                } else {
                    "此操作已被系统禁止。"
                };
                return Some(ToolOutcome {
                    call_id: call.id.clone(),
                    status: ToolStatus::HardBlocked { reason: reason.into() },
                    content: format!("exec_shell 已被系统禁止：{}", reason),
                    was_truncated: false,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*; use serde_json::json;
    #[test] fn test_blocks() {
        let g = SafetyGuard::new();
        assert!(g.check(&ToolCall { id: "1".into(), name: "exec_shell".into(), arguments: json!({"cmd":"Remove-Item -Recurse -Force x"}) }).is_some());
        assert!(g.check(&ToolCall { id: "2".into(), name: "exec_shell".into(), arguments: json!({"cmd":"Write-Output hi"}) }).is_none());
        assert!(g.check(&ToolCall { id: "3".into(), name: "read_file".into(), arguments: json!({"path":"x"}) }).is_none());
    }
}
