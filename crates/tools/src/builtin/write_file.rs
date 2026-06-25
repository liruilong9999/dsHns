use std::{collections::HashMap, path::PathBuf};
use async_trait::async_trait;
use dshns_core::tool::*;

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("path".into(), ParamProp { prop_type: "string".into(), description: "文件绝对路径".into(), enum_values: None });
        props.insert("content".into(), ParamProp { prop_type: "string".into(), description: "要写入的内容".into(), enum_values: None });
        ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: "write_file".into(),
                description: "将内容写入指定文件(覆盖模式)。UTF-8 无 BOM 编码。".into(),
                parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["path".into(), "content".into()] },
            },
        }
    }

    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let path = PathBuf::from(call.arguments["path"].as_str().unwrap_or(""));
        let content = call.arguments["content"].as_str().unwrap_or("");
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).ok(); }
        match std::fs::write(&path, content) {
            Ok(_) => ToolOutcome {
                call_id: call.id.clone(), status: ToolStatus::Success,
                content: format!("成功写入 {} ({}行, {}字节, UTF-8)", path.display(), content.lines().count(), content.len()),
                was_truncated: false,
            },
            Err(e) => ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Error { reason: e.to_string() }, content: String::new(), was_truncated: false },
        }
    }
}
