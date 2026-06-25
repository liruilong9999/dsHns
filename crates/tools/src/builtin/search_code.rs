use std::{collections::HashMap, path::PathBuf, process::Stdio};
use async_trait::async_trait;
use dshns_core::tool::*;

pub struct SearchCodeTool;

#[async_trait]
impl Tool for SearchCodeTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("pattern".into(), ParamProp { prop_type: "string".into(), description: "正则表达式模式".into(), enum_values: None });
        props.insert("path".into(), ParamProp { prop_type: "string".into(), description: "搜索目录(可选)".into(), enum_values: None });
        props.insert("glob".into(), ParamProp { prop_type: "string".into(), description: "文件类型过滤如*.rs(可选)".into(), enum_values: None });
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "search_code".into(),
                description: "使用 ripgrep 在代码中搜索正则匹配。支持文件类型过滤。".into(),
                parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["pattern".into()] },
            },
        }
    }

    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let pattern = call.arguments["pattern"].as_str().unwrap_or("");
        let search_path = call.arguments["path"].as_str().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let glob = call.arguments["glob"].as_str();
        let mut cmd = tokio::process::Command::new("rg");
        cmd.args(["--line-number", "--color", "never", "--no-heading", pattern])
            .arg(&search_path).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(g) = glob { cmd.args(["--glob", g]); }
        cmd.arg("-m").arg("200");

        match cmd.output().await {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                if stdout.trim().is_empty() {
                    ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: "未找到匹配结果".into(), was_truncated: false }
                } else {
                    let lines = stdout.lines().count();
                    ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: format!("找到{}行:\n{}", lines, stdout), was_truncated: lines > 200 }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: "ripgrep (rg) 未安装。winget install BurntSushi.ripgrep.MSVC".into() }, content: String::new(), was_truncated: false }
            }
            Err(e) => ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: e.to_string() }, content: String::new(), was_truncated: false },
        }
    }
}
