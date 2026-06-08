//! 工具注册表实现。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domain::ToolRiskLevel;
use crate::skill::manager::SkillManager;
use crate::tools::builtin::{
    AgentCloseTool, AgentEvalTool, AgentOpenTool, AutomationCreateTool, AutomationRunOnceTool,
    CallMcpTool, ConnectMcpServerTool, DiscoverMcpServersTool, FetchUrlTool, GithubGetTool,
    LoadSkillTool, PlanWriteTool, ReadFileTool, ReadToolResultTool, RlmCloseTool, RlmConfigureTool,
    RlmEvalTool, RlmOpenTool, RunShellTool, TaskCreateTool, TaskRunTool, WebRunTool, WebSearchTool,
    WriteFileTool,
};

/// 工具定义。
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// 工具名称。
    pub name: String,
    /// 工具说明。
    pub description: String,
    /// 参数 Schema。
    pub parameters_schema: Value,
    /// 风险等级。
    pub risk_level: ToolRiskLevel,
    /// 是否向模型暴露。
    pub visible_to_model: bool,
}

/// 工具执行上下文。
#[derive(Debug, Clone)]
pub struct ToolExecutionContext {
    /// 工作区根目录。
    pub workspace_root: PathBuf,
    /// 当前会话目录。
    pub session_dir: PathBuf,
    /// Shell 程序名。
    pub shell_program: String,
    /// Skill 管理器。
    pub skill_manager: SkillManager,
}

/// 工具处理器接口。
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// 执行工具。
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String>;
}

/// 已注册工具。
pub struct RegisteredTool {
    /// 工具定义。
    pub definition: ToolDefinition,
    /// 工具处理器。
    pub handler: Arc<dyn ToolHandler>,
}

/// 工具注册表。
pub struct ToolRegistry {
    /// 工具映射表。
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    /// 创建默认注册表。
    pub fn with_defaults() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };

        registry.register(
            ToolDefinition {
                name: "read_file".to_string(),
                description: "读取 UTF-8 文本文件内容。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "目标文件路径" }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::ReadOnly,
                visible_to_model: true,
            },
            Arc::new(ReadFileTool),
        );

        registry.register(
            ToolDefinition {
                name: "write_file".to_string(),
                description: "写入文件，支持整文件覆盖或 replace_range 按行替换。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "目标文件路径" },
                        "content": { "type": "string", "description": "整文件写入内容" },
                        "replace_range": {
                            "type": "object",
                            "properties": {
                                "start_line": { "type": "integer" },
                                "end_line": { "type": "integer" },
                                "new_content": { "type": "string" }
                            },
                            "required": ["start_line", "end_line", "new_content"],
                            "additionalProperties": false
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Write,
                visible_to_model: true,
            },
            Arc::new(WriteFileTool),
        );

        registry.register(
            ToolDefinition {
                name: "run_shell".to_string(),
                description: "在 PowerShell 中执行命令。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "待执行命令" },
                        "working_directory": { "type": "string", "description": "执行目录" },
                        "timeout_ms": { "type": "integer", "description": "超时时间，毫秒" }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(RunShellTool),
        );

        registry.register(
            ToolDefinition {
                name: "rlm_open".to_string(),
                description: "启动一个可复用的持久交互进程。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "working_directory": { "type": "string", "description": "可选工作目录" }
                    },
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(RlmOpenTool),
        );

        registry.register(
            ToolDefinition {
                name: "rlm_eval".to_string(),
                description: "向持久交互进程发送命令并读取输出。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "process_id": { "type": "string", "description": "持久进程标识" },
                        "command": { "type": "string", "description": "待执行命令" }
                    },
                    "required": ["process_id", "command"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(RlmEvalTool),
        );

        registry.register(
            ToolDefinition {
                name: "rlm_configure".to_string(),
                description: "调整持久交互进程配置，例如切换工作目录。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "process_id": { "type": "string", "description": "持久进程标识" },
                        "working_directory": { "type": "string", "description": "新的工作目录" }
                    },
                    "required": ["process_id"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(RlmConfigureTool),
        );

        registry.register(
            ToolDefinition {
                name: "rlm_close".to_string(),
                description: "关闭持久交互进程并释放资源。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "process_id": { "type": "string", "description": "持久进程标识" }
                    },
                    "required": ["process_id"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(RlmCloseTool),
        );

        registry.register(
            ToolDefinition {
                name: "load_skill".to_string(),
                description: "按名称或路径加载目标 Skill.md 正文。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "identifier": { "type": "string", "description": "Skill 名称或路径" }
                    },
                    "required": ["identifier"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::ReadOnly,
                visible_to_model: true,
            },
            Arc::new(LoadSkillTool),
        );

        registry.register(
            ToolDefinition {
                name: "read_tool_result".to_string(),
                description: "根据 tool:{id} 句柄读取已外置的工具结果。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "handle": { "type": "string", "description": "工具结果句柄，例如 tool:call_xxx" }
                    },
                    "required": ["handle"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::ReadOnly,
                visible_to_model: true,
            },
            Arc::new(ReadToolResultTool),
        );

        registry.register(
            ToolDefinition {
                name: "fetch_url".to_string(),
                description: "抓取指定 URL 的文本内容。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "目标 URL" }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Network,
                visible_to_model: true,
            },
            Arc::new(FetchUrlTool),
        );

        registry.register(
            ToolDefinition {
                name: "web_search".to_string(),
                description: "执行网络搜索并返回结构化结果列表。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "搜索关键字" },
                        "limit": { "type": "integer", "description": "返回结果数量上限" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Network,
                visible_to_model: true,
            },
            Arc::new(WebSearchTool),
        );

        registry.register(
            ToolDefinition {
                name: "web_run".to_string(),
                description: "执行基础网页步骤，包括 open、click、find、extract_text。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "steps": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "action": { "type": "string" },
                                    "url": { "type": "string" },
                                    "text_contains": { "type": "string" },
                                    "pattern": { "type": "string" },
                                    "selector": { "type": "string" }
                                },
                                "required": ["action"],
                                "additionalProperties": true
                            }
                        }
                    },
                    "required": ["steps"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Network,
                visible_to_model: true,
            },
            Arc::new(WebRunTool),
        );

        registry.register(
            ToolDefinition {
                name: "github_get".to_string(),
                description: "调用 GitHub REST API 的只读 GET 接口。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "endpoint": { "type": "string", "description": "GitHub API 相对路径，例如 /repos/owner/repo/issues" }
                    },
                    "required": ["endpoint"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Network,
                visible_to_model: true,
            },
            Arc::new(GithubGetTool),
        );

        registry.register(
            ToolDefinition {
                name: "discover_mcp_servers".to_string(),
                description: "发现并列出当前工作区可用的 MCP 服务端配置。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::ReadOnly,
                visible_to_model: true,
            },
            Arc::new(DiscoverMcpServersTool),
        );

        registry.register(
            ToolDefinition {
                name: "connect_mcp_server".to_string(),
                description: "连接指定 MCP 服务端并读取能力信息。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "server_id": { "type": "string", "description": "MCP 服务端标识" }
                    },
                    "required": ["server_id"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Network,
                visible_to_model: true,
            },
            Arc::new(ConnectMcpServerTool),
        );

        registry.register(
            ToolDefinition {
                name: "call_mcp_tool".to_string(),
                description: "调用指定 MCP 服务端上的远程工具。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "server_id": { "type": "string", "description": "MCP 服务端标识" },
                        "tool_name": { "type": "string", "description": "远程工具名称" },
                        "arguments": { "type": "object", "description": "远程工具参数" }
                    },
                    "required": ["server_id", "tool_name", "arguments"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Network,
                visible_to_model: true,
            },
            Arc::new(CallMcpTool),
        );

        registry.register(
            ToolDefinition {
                name: "agent_open".to_string(),
                description: "创建子 Agent，并记录隔离或继承模式约束。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "mode": { "type": "string", "description": "inherit 或 isolate" },
                        "inherit_context": { "type": "boolean", "description": "是否继承上下文" },
                        "allowed_paths": { "type": "array", "items": { "type": "string" } },
                        "task_spec": { "type": "object", "description": "任务约束快照" },
                        "parent_agent_id": { "type": "string", "description": "可选父 Agent 标识" }
                    },
                    "required": ["mode", "task_spec"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Agent,
                visible_to_model: true,
            },
            Arc::new(AgentOpenTool),
        );

        registry.register(
            ToolDefinition {
                name: "agent_eval".to_string(),
                description: "向已创建的子 Agent 派发执行请求。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "子 Agent 标识" },
                        "input": { "description": "执行输入" }
                    },
                    "required": ["agent_id", "input"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Agent,
                visible_to_model: true,
            },
            Arc::new(AgentEvalTool),
        );

        registry.register(
            ToolDefinition {
                name: "agent_close".to_string(),
                description: "关闭指定子 Agent。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "子 Agent 标识" }
                    },
                    "required": ["agent_id"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Agent,
                visible_to_model: true,
            },
            Arc::new(AgentCloseTool),
        );

        registry.register(
            ToolDefinition {
                name: "plan_write".to_string(),
                description: "写入或更新当前会话下的计划文档。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "plan_type": { "type": "string", "description": "计划类型，例如 implementation 或 review" },
                        "content": { "type": "string", "description": "计划正文内容" }
                    },
                    "required": ["plan_type", "content"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Write,
                visible_to_model: true,
            },
            Arc::new(PlanWriteTool),
        );

        registry.register(
            ToolDefinition {
                name: "task_create".to_string(),
                description: "创建会话内任务记录。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "任务名称" },
                        "command": { "type": "string", "description": "任务命令" },
                        "validation_command": { "type": "string", "description": "可选验证命令" }
                    },
                    "required": ["name", "command"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Write,
                visible_to_model: true,
            },
            Arc::new(TaskCreateTool),
        );

        registry.register(
            ToolDefinition {
                name: "task_run".to_string(),
                description: "执行已创建的会话任务。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "task_id": { "type": "string", "description": "任务标识" },
                        "timeout_ms": { "type": "integer", "description": "可选超时时间" }
                    },
                    "required": ["task_id"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(TaskRunTool),
        );

        registry.register(
            ToolDefinition {
                name: "automation_create".to_string(),
                description: "创建自动化记录。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "自动化名称" },
                        "kind": { "type": "string", "description": "自动化类型" },
                        "status": { "type": "string", "description": "状态，例如 active 或 paused" },
                        "definition": { "type": "object", "description": "自动化定义" }
                    },
                    "required": ["name", "kind", "status", "definition"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Write,
                visible_to_model: true,
            },
            Arc::new(AutomationCreateTool),
        );

        registry.register(
            ToolDefinition {
                name: "automation_run_once".to_string(),
                description: "立刻执行一次自动化。".to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "automation_id": { "type": "string", "description": "自动化标识" },
                        "timeout_ms": { "type": "integer", "description": "可选超时时间" }
                    },
                    "required": ["automation_id"],
                    "additionalProperties": false
                }),
                risk_level: ToolRiskLevel::Execute,
                visible_to_model: true,
            },
            Arc::new(AutomationRunOnceTool),
        );

        registry
    }

    /// 注册单个工具。
    pub fn register(&mut self, definition: ToolDefinition, handler: Arc<dyn ToolHandler>) {
        self.tools.insert(
            definition.name.clone(),
            RegisteredTool {
                definition,
                handler,
            },
        );
    }

    /// 按名称获取工具定义与处理器。
    pub fn get(&self, name: &str) -> Result<&RegisteredTool> {
        self.tools
            .get(name)
            .ok_or_else(|| anyhow!("未注册的工具：{}", name))
    }

    /// 返回对模型可见的工具定义列表。
    pub fn model_tools(&self) -> Vec<Value> {
        self.tools
            .values()
            .filter(|tool| tool.definition.visible_to_model)
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.definition.name,
                        "description": tool.definition.description,
                        "parameters": tool.definition.parameters_schema
                    }
                })
            })
            .collect()
    }
}
