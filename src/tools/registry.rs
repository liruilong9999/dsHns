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
    CallMcpTool, ConnectMcpServerTool, DiscoverMcpServersTool, EditFileTool, FetchUrlTool,
    FileSearchTool, GithubGetTool, GrepFilesTool, HandleReadTool, ListDirTool, LoadSkillTool,
    PlanWriteTool, ReadFileTool, ReadToolResultTool, RetrieveToolResultTool, RlmCloseTool,
    RlmConfigureTool, RlmEvalTool, RlmOpenTool, RunShellTool, TaskCreateTool, TaskRunTool,
    WebRunTool, WebSearchTool, WriteFileTool,
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
            tool_definition(
                "read_file",
                "读取 UTF-8 文本文件内容。",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "目标文件路径" }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(ReadFileTool),
        );
        registry.register(
            tool_definition(
                "write_file",
                "写入文件，支持整文件覆盖或 replace_range 按行替换。",
                json!({
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
                ToolRiskLevel::Write,
            ),
            Arc::new(WriteFileTool),
        );
        registry.register(
            tool_definition(
                "run_shell",
                "在 PowerShell 中执行命令。",
                json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "待执行命令" },
                        "working_directory": { "type": "string", "description": "执行目录" },
                        "timeout_ms": { "type": "integer", "description": "超时时间，毫秒" }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
            Arc::new(RunShellTool),
        );
        registry.register(
            tool_definition(
                "edit_file",
                "单文件查找替换，支持 replace_all。",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "目标文件路径" },
                        "old_text": { "type": "string", "description": "待替换旧文本" },
                        "new_text": { "type": "string", "description": "新文本" },
                        "replace_all": { "type": "boolean", "description": "是否替换全部匹配" }
                    },
                    "required": ["path", "old_text", "new_text"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Write,
            ),
            Arc::new(EditFileTool),
        );
        registry.register(
            tool_definition(
                "list_dir",
                "列目录，支持隐藏文件开关。",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "可选目录路径" },
                        "include_hidden": { "type": "boolean", "description": "是否包含隐藏文件" }
                    },
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(ListDirTool),
        );
        registry.register(
            tool_definition(
                "file_search",
                "按文件名关键字递归搜索。",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "文件名关键字" },
                        "root_path": { "type": "string", "description": "可选搜索根目录" },
                        "limit": { "type": "integer", "description": "可选返回数量上限" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(FileSearchTool),
        );
        registry.register(
            tool_definition(
                "grep_files",
                "在工作区递归按正则搜索内容。",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "正则表达式" },
                        "root_path": { "type": "string", "description": "可选搜索根目录" },
                        "limit": { "type": "integer", "description": "可选返回数量上限" }
                    },
                    "required": ["pattern"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(GrepFilesTool),
        );
        registry.register(
            tool_definition(
                "rlm_open",
                "启动一个可复用的持久交互进程。",
                json!({
                    "type": "object",
                    "properties": {
                        "working_directory": { "type": "string", "description": "可选工作目录" }
                    },
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
            Arc::new(RlmOpenTool),
        );
        registry.register(
            tool_definition(
                "rlm_eval",
                "向持久交互进程发送命令并读取输出。",
                json!({
                    "type": "object",
                    "properties": {
                        "process_id": { "type": "string", "description": "持久进程标识" },
                        "command": { "type": "string", "description": "待执行命令" }
                    },
                    "required": ["process_id", "command"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
            Arc::new(RlmEvalTool),
        );
        registry.register(
            tool_definition(
                "rlm_configure",
                "调整持久交互进程配置，例如切换工作目录。",
                json!({
                    "type": "object",
                    "properties": {
                        "process_id": { "type": "string", "description": "持久进程标识" },
                        "working_directory": { "type": "string", "description": "新的工作目录" }
                    },
                    "required": ["process_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
            Arc::new(RlmConfigureTool),
        );
        registry.register(
            tool_definition(
                "rlm_close",
                "关闭持久交互进程并释放资源。",
                json!({
                    "type": "object",
                    "properties": {
                        "process_id": { "type": "string", "description": "持久进程标识" }
                    },
                    "required": ["process_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
            Arc::new(RlmCloseTool),
        );
        registry.register(
            tool_definition(
                "load_skill",
                "按名称或路径加载目标 SKILL.md 正文。",
                json!({
                    "type": "object",
                    "properties": {
                        "identifier": { "type": "string", "description": "Skill 名称或路径" }
                    },
                    "required": ["identifier"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(LoadSkillTool),
        );
        registry.register(
            tool_definition(
                "read_tool_result",
                "根据 tool:{id} 句柄读取已外置的工具结果。",
                json!({
                    "type": "object",
                    "properties": {
                        "handle": { "type": "string", "description": "工具结果句柄，例如 tool:call_xxx" }
                    },
                    "required": ["handle"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(ReadToolResultTool),
        );
        registry.register(
            tool_definition(
                "retrieve_tool_result",
                "按工具调用编号读取摘要、头部、尾部、切片或关键字上下文。",
                json!({
                    "type": "object",
                    "properties": {
                        "tool_call_id": { "type": "string", "description": "工具调用编号" },
                        "mode": { "type": "string", "description": "summary、head、tail、body、slice、keyword_context" },
                        "start_char": { "type": "integer", "description": "slice 模式下的起始字符偏移" },
                        "length_chars": { "type": "integer", "description": "slice 模式下的读取字符数" },
                        "keyword": { "type": "string", "description": "keyword_context 模式下的关键字" },
                        "context_chars": { "type": "integer", "description": "关键字前后保留的字符数" }
                    },
                    "required": ["tool_call_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(RetrieveToolResultTool),
        );
        registry.register(
            tool_definition(
                "handle_read",
                "按 tool: 或 file: 句柄读取受限内容。",
                json!({
                    "type": "object",
                    "properties": {
                        "handle": { "type": "string", "description": "tool: 或 file: 句柄" },
                        "max_chars": { "type": "integer", "description": "可选最大返回字符数" }
                    },
                    "required": ["handle"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(HandleReadTool),
        );
        registry.register(
            tool_definition(
                "fetch_url",
                "抓取指定 URL 的文本内容。",
                json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "目标 URL" }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Network,
            ),
            Arc::new(FetchUrlTool),
        );
        registry.register(
            tool_definition(
                "web_search",
                "执行网络搜索并返回结构化结果列表。",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "搜索关键字" },
                        "limit": { "type": "integer", "description": "返回结果数量上限" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Network,
            ),
            Arc::new(WebSearchTool),
        );
        registry.register(
            tool_definition(
                "web_run",
                "执行基础网页步骤，包括 open、click、find、extract_text。",
                json!({
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
                ToolRiskLevel::Network,
            ),
            Arc::new(WebRunTool),
        );
        registry.register(
            tool_definition(
                "github_get",
                "调用 GitHub REST API 的只读 GET 接口。",
                json!({
                    "type": "object",
                    "properties": {
                        "endpoint": { "type": "string", "description": "GitHub API 相对路径，例如 /repos/owner/repo/issues" }
                    },
                    "required": ["endpoint"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Network,
            ),
            Arc::new(GithubGetTool),
        );
        registry.register(
            tool_definition(
                "discover_mcp_servers",
                "发现并列出当前工作区可用的 MCP 服务端配置。",
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                ToolRiskLevel::ReadOnly,
            ),
            Arc::new(DiscoverMcpServersTool),
        );
        registry.register(
            tool_definition(
                "connect_mcp_server",
                "连接指定 MCP 服务端并读取能力信息。",
                json!({
                    "type": "object",
                    "properties": {
                        "server_id": { "type": "string", "description": "MCP 服务端标识" }
                    },
                    "required": ["server_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Network,
            ),
            Arc::new(ConnectMcpServerTool),
        );
        registry.register(
            tool_definition(
                "call_mcp_tool",
                "调用指定 MCP 服务端上的远程工具。",
                json!({
                    "type": "object",
                    "properties": {
                        "server_id": { "type": "string", "description": "MCP 服务端标识" },
                        "tool_name": { "type": "string", "description": "远程工具名称" },
                        "arguments": { "type": "object", "description": "远程工具参数" }
                    },
                    "required": ["server_id", "tool_name", "arguments"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Network,
            ),
            Arc::new(CallMcpTool),
        );
        registry.register(
            tool_definition(
                "agent_open",
                "创建子 Agent，并记录隔离或继承模式约束。",
                json!({
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
                ToolRiskLevel::Agent,
            ),
            Arc::new(AgentOpenTool),
        );
        registry.register(
            tool_definition(
                "agent_eval",
                "向已创建的子 Agent 派发执行请求。",
                json!({
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "子 Agent 标识" },
                        "input": { "description": "执行输入" }
                    },
                    "required": ["agent_id", "input"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Agent,
            ),
            Arc::new(AgentEvalTool),
        );
        registry.register(
            tool_definition(
                "agent_close",
                "关闭指定子 Agent。",
                json!({
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "子 Agent 标识" }
                    },
                    "required": ["agent_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Agent,
            ),
            Arc::new(AgentCloseTool),
        );
        registry.register(
            tool_definition(
                "plan_write",
                "写入或更新当前会话下的计划文档。",
                json!({
                    "type": "object",
                    "properties": {
                        "plan_type": { "type": "string", "description": "计划类型，例如 implementation 或 review" },
                        "content": { "type": "string", "description": "计划正文内容" }
                    },
                    "required": ["plan_type", "content"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Write,
            ),
            Arc::new(PlanWriteTool),
        );
        registry.register(
            tool_definition(
                "task_create",
                "创建会话内任务记录。",
                json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "任务名称" },
                        "command": { "type": "string", "description": "任务命令" },
                        "validation_command": { "type": "string", "description": "可选验证命令" }
                    },
                    "required": ["name", "command"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Write,
            ),
            Arc::new(TaskCreateTool),
        );
        registry.register(
            tool_definition(
                "task_run",
                "执行已创建的会话任务。",
                json!({
                    "type": "object",
                    "properties": {
                        "task_id": { "type": "string", "description": "任务标识" },
                        "timeout_ms": { "type": "integer", "description": "可选超时时间" }
                    },
                    "required": ["task_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
            Arc::new(TaskRunTool),
        );
        registry.register(
            tool_definition(
                "automation_create",
                "创建自动化记录。",
                json!({
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
                ToolRiskLevel::Write,
            ),
            Arc::new(AutomationCreateTool),
        );
        registry.register(
            tool_definition(
                "automation_run_once",
                "立刻执行一次自动化。",
                json!({
                    "type": "object",
                    "properties": {
                        "automation_id": { "type": "string", "description": "自动化标识" },
                        "timeout_ms": { "type": "integer", "description": "可选超时时间" }
                    },
                    "required": ["automation_id"],
                    "additionalProperties": false
                }),
                ToolRiskLevel::Execute,
            ),
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
        self.model_tools_for_context("", false, true, true)
    }

    /// 根据当前上下文和配置返回对模型可见的工具列表。
    pub fn model_tools_for_context(
        &self,
        context_text: &str,
        enable_adaptive_tool_exposure: bool,
        allow_network: bool,
        allow_plugin_tool: bool,
    ) -> Vec<Value> {
        let normalized = context_text.to_ascii_lowercase();
        let github_enabled = allow_network && std::env::var("GITHUB_TOKEN").is_ok();

        self.tools
            .values()
            .filter(|tool| tool.definition.visible_to_model)
            .filter(|tool| {
                if !enable_adaptive_tool_exposure {
                    return if tool.definition.name == "github_get" {
                        github_enabled
                    } else if is_network_tool(&tool.definition.name) {
                        allow_network
                    } else {
                        true
                    };
                }

                match tool.definition.name.as_str() {
                    "fetch_url" | "web_search" | "web_run" => allow_network,
                    "github_get" => {
                        github_enabled
                            && contains_any_keyword(&normalized, &["github", "pr", "issue"])
                    }
                    "automation_create" | "automation_run_once" => contains_any_keyword(
                        &normalized,
                        &[
                            "automation",
                            "cron",
                            "heartbeat",
                            "\u{81ea}\u{52a8}\u{5316}",
                            "\u{5b9a}\u{65f6}",
                        ],
                    ),
                    "agent_open" | "agent_eval" | "agent_close" => {
                        allow_plugin_tool
                            && contains_any_keyword(
                                &normalized,
                                &[
                                    "agent",
                                    "subagent",
                                    "\u{5b50}\u{4ee3}\u{7406}",
                                    "\u{5b50} agent",
                                ],
                            )
                    }
                    "rlm_open" | "rlm_eval" | "rlm_configure" | "rlm_close" => {
                        contains_any_keyword(&normalized, &["rlm", "\u{63a8}\u{7406}\u{673a}"])
                    }
                    _ => true,
                }
            })
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

fn tool_definition(
    name: &str,
    description: &str,
    parameters_schema: Value,
    risk_level: ToolRiskLevel,
) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        parameters_schema,
        risk_level,
        visible_to_model: true,
    }
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn is_network_tool(name: &str) -> bool {
    matches!(name, "fetch_url" | "web_search" | "web_run" | "github_get")
}

#[cfg(test)]
mod tests {
    use super::ToolRegistry;

    fn tool_names(tools: Vec<serde_json::Value>) -> Vec<String> {
        tools
            .into_iter()
            .filter_map(|tool| {
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .map(|value| value.to_string())
            })
            .collect()
    }

    #[test]
    fn should_filter_network_tools_when_network_disabled() {
        let registry = ToolRegistry::with_defaults();
        let names = tool_names(registry.model_tools_for_context("search rust", true, false, true));
        assert!(!names.iter().any(|name| name == "web_search"));
        assert!(!names.iter().any(|name| name == "fetch_url"));
        assert!(names.iter().any(|name| name == "read_file"));
    }

    #[test]
    fn should_expose_contextual_tools_when_keywords_match() {
        let registry = ToolRegistry::with_defaults();
        let names = tool_names(registry.model_tools_for_context(
            "需要一个 subagent 做 automation cron，并用 rlm 推理机",
            true,
            true,
            true,
        ));
        assert!(names.iter().any(|name| name == "agent_open"));
        assert!(names.iter().any(|name| name == "automation_create"));
        assert!(names.iter().any(|name| name == "rlm_open"));
    }

    #[test]
    fn should_only_expose_github_tool_with_token_and_keyword() {
        let registry = ToolRegistry::with_defaults();
        std::env::remove_var("GITHUB_TOKEN");
        let without_token =
            tool_names(registry.model_tools_for_context("github issue", true, true, true));
        assert!(!without_token.iter().any(|name| name == "github_get"));

        std::env::set_var("GITHUB_TOKEN", "demo");
        let with_token =
            tool_names(registry.model_tools_for_context("github issue", true, true, true));
        assert!(with_token.iter().any(|name| name == "github_get"));
        std::env::remove_var("GITHUB_TOKEN");
    }
}
