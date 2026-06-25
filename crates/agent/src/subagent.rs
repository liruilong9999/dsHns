use std::collections::HashMap;
use async_trait::async_trait;
use dshns_core::tool::*;

pub struct AgentOpenTool;

#[async_trait]
impl Tool for AgentOpenTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("mode".into(), ParamProp { prop_type: "string".into(), description: "inherit 或 isolated".into(), enum_values: Some(vec!["inherit".into(), "isolated".into()]) });
        props.insert("prompt".into(), ParamProp { prop_type: "string".into(), description: "子智能体的任务描述".into(), enum_values: None });
        props.insert("description".into(), ParamProp { prop_type: "string".into(), description: "简短描述(3-5字)".into(), enum_values: None });
        ToolDef { tool_type: "function".into(), function: FunctionDef { name: "agent_open".into(), description: "创建子智能体执行独立任务。子智能体完成后通过 agent_result 汇报。子智能体不可创建子智能体。".into(), parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["mode".into(), "prompt".into()] } } }
    }
    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let prompt = call.arguments["prompt"].as_str().unwrap_or("unknown");
        ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: format!("[子智能体] 任务: {}\n(子智能体功能待完善)", prompt), was_truncated: false }
    }
}

pub struct AgentCloseTool;

#[async_trait]
impl Tool for AgentCloseTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("agent_id".into(), ParamProp { prop_type: "string".into(), description: "子智能体 ID".into(), enum_values: None });
        ToolDef { tool_type: "function".into(), function: FunctionDef { name: "agent_close".into(), description: "强制终止子智能体".into(), parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["agent_id".into()] } } }
    }
    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: "子智能体已关闭".into(), was_truncated: false }
    }
}

pub struct AgentResultTool;

#[async_trait]
impl Tool for AgentResultTool {
    fn definition(&self) -> ToolDef {
        let mut props = HashMap::new();
        props.insert("result".into(), ParamProp { prop_type: "string".into(), description: "任务完成总结".into(), enum_values: None });
        ToolDef { tool_type: "function".into(), function: FunctionDef { name: "agent_result".into(), description: "子智能体汇报完成结果。调用后子智能体结束。".into(), parameters: ToolParams { param_type: "object".into(), properties: props, required: vec!["result".into()] } } }
    }
    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let result = call.arguments["result"].as_str().unwrap_or("完成");
        ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Success, content: format!("[子智能体结果]\n{}", result), was_truncated: false }
    }
}
