use std::collections::HashMap;
use std::path::PathBuf;
use async_trait::async_trait;
use dshns_core::tool::*;

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("path".into(), ParamProp { prop_type: "string".into(), description: "文件绝对路径".into(), enum_values: None });
        props.insert("offset".into(), ParamProp { prop_type: "number".into(), description: "起始行号(可选)".into(), enum_values: None });
        props.insert("limit".into(), ParamProp { prop_type: "number".into(), description: "读取行数(可选)".into(), enum_values: None });
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "read_file".into(),
                description: "读取指定路径的文件内容，支持指定起始行和行数。超出2000行建议用 offset/limit。".into(),
                parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["path".into()] },
            },
        }
    }

    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let path = match call.arguments["path"].as_str() {
            Some(p) => PathBuf::from(p),
            None => return ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: "缺少 path".into() }, content: String::new(), was_truncated: false },
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let total = lines.len();
                let offset = call.arguments["offset"].as_u64().unwrap_or(0) as usize;
                let limit = call.arguments["limit"].as_u64().map(|l| l as usize);
                let start = offset.min(total);
                let end = limit.map(|l| (start+l).min(total)).unwrap_or(total);
                let selected: Vec<String> = lines[start..end].iter().enumerate()
                    .map(|(i,l)| format!("{:>6}\t{}", start+i+1, l)).collect();
                let result = if start > 0 || end < total {
                    format!("文件 {} (第{}-{}行/共{}行):\n{}", path.display(), start+1, end, total, selected.join("\n"))
                } else {
                    format!("文件 {} ({}行):\n{}", path.display(), total, selected.join("\n"))
                };
                ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: result, was_truncated: false }
            }
            Err(e) => ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: format!("读取失败: {}", e) }, content: String::new(), was_truncated: false },
        }
    }
}
