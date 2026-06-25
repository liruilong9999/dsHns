use std::{collections::HashMap, time::Duration};
use async_trait::async_trait;
use dshns_core::tool::*;

pub struct ExecShellTool { timeout_secs: u64 }

impl ExecShellTool {
    pub fn new(timeout_secs: u64) -> Self { Self { timeout_secs } }

    pub fn check_hard_block(cmd: &str) -> Option<&'static str> {
        let cmd_lower = cmd.to_lowercase();
        // Remove-Item -Recurse -Force
        if (cmd_lower.contains("remove-item") || cmd_lower.contains("rm"))
            && cmd_lower.contains("recurse") && cmd_lower.contains("force") {
            return Some("递归强制删除已被系统禁止。请使用 Remove-Item <具体文件路径> 逐个删除。");
        }
        // del /f /s /q or rd /s /q
        if (cmd_lower.contains("del") || cmd_lower.contains("rd"))
            && cmd_lower.contains("/f") && cmd_lower.contains("/s") {
            return Some("递归强制删除已被系统禁止。请逐个指定文件路径。");
        }
        // runas
        if cmd_lower.contains("runas") || cmd_lower.contains("start-process -verb runas") {
            return Some("提权操作已被系统禁止。");
        }
        // format/diskpart
        if cmd_lower.starts_with("format ") || cmd_lower.contains("diskpart") {
            return Some("磁盘操作已被系统禁止。");
        }
        None
    }
}

#[async_trait]
impl Tool for ExecShellTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("cmd".into(), ParamProp { prop_type: "string".into(), description: "要执行的 PowerShell 命令".into(), enum_values: None });
        props.insert("cwd".into(), ParamProp { prop_type: "string".into(), description: "工作目录(可选)".into(), enum_values: None });
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "exec_shell".into(),
                description: "执行 PowerShell 命令(powershell.exe -NoProfile)。编码 UTF-8(cp65001)。危险命令会被系统拒绝。".into(),
                parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["cmd".into()] },
            },
        }
    }

    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let cmd = match call.arguments["cmd"].as_str() {
            Some(c) => c,
            None => return ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: "缺少 cmd".into() }, content: String::new(), was_truncated: false },
        };
        if let Some(reason) = Self::check_hard_block(cmd) {
            return ToolOutcome { call_id: call.id.clone(), status: ToolStatus::HardBlocked { reason: reason.into() },
                content: format!("exec_shell 已被系统禁止：{}。请使用其他方式。", reason), was_truncated: false };
        }
        let cwd = call.arguments["cwd"].as_str().map(std::path::PathBuf::from);
        let wrapped = format!("chcp 65001 > $null; {}", cmd);
        let mut command = tokio::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-Command", &wrapped]);
        if let Some(ref d) = cwd { command.current_dir(d); }

        let output = match tokio::time::timeout(Duration::from_secs(self.timeout_secs), command.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: e.to_string() }, content: String::new(), was_truncated: false },
            Err(_) => return ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Timeout, content: format!("超时({}s)", self.timeout_secs), was_truncated: false },
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if output.status.success() {
            ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: if stdout.is_empty() { "(无输出)".into() } else { stdout }, was_truncated: false }
        } else {
            let msg = format!("退出码: {}\nstdout:\n{}\nstderr:\n{}", output.status.code().unwrap_or(-1), stdout, stderr);
            ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: msg.clone() }, content: msg, was_truncated: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_hard_block_cases() {
        assert!(ExecShellTool::check_hard_block("Remove-Item -Recurse -Force foo").is_some());
        assert!(ExecShellTool::check_hard_block("del /f /s /q foo").is_some());
        assert!(ExecShellTool::check_hard_block("runas notepad").is_some());
        assert!(ExecShellTool::check_hard_block("Write-Output hello").is_none());
        assert!(ExecShellTool::check_hard_block("Remove-Item foo.txt").is_none());
    }
}
