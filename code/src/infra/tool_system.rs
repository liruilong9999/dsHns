//! 工具系统实现。
//!
//! 本模块负责默认工具注册、参数校验、审批矩阵、边界控制与执行器分发。

use crate::domain::tool::{
    SessionApprovalMode, ToolCallRequest, ToolMetadata, ToolPermission, ToolResponse,
    ToolSchemaNode, ToolValueType,
};
use crate::infra::agent_management::{
    ChildAgentDispatchRequest, ChildAgentManager, ChildAgentMode, CreateChildAgentRequest,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 工具运行时配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRuntimeConfig {
    /// 当前工作区根目录。
    pub workspace_root_path: PathBuf,
    /// 技能根目录。
    pub skill_root_path: PathBuf,
    /// 单轮工具最大调用次数。
    pub max_tool_calls_per_round: u32,
    /// 同一工具连续失败熔断阈值。
    pub tool_failure_streak_limit: u32,
}

impl ToolRuntimeConfig {
    /// 构造工具运行时配置。
    pub fn new(workspace_root_path: PathBuf, skill_root_path: PathBuf) -> Self {
        Self {
            workspace_root_path,
            skill_root_path,
            max_tool_calls_per_round: 12,
            tool_failure_streak_limit: 5,
        }
    }
}

/// 工具注册中心。
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    /// 工具元数据映射。
    metadata_map: HashMap<String, ToolMetadata>,
}

impl ToolRegistry {
    /// 构造默认工具注册中心。
    pub fn with_default_tools() -> Self {
        let mut metadata_map = HashMap::new();
        for metadata in [
            default_run_shell_metadata(),
            default_read_file_metadata(),
            default_write_file_metadata(),
            default_load_skill_metadata(),
            default_plan_tool_metadata(),
            default_create_agent_metadata(),
        ] {
            metadata_map.insert(metadata.name.clone(), metadata);
        }

        Self { metadata_map }
    }

    /// 按工具名获取元数据。
    pub fn metadata(&self, tool_name: &str) -> Option<&ToolMetadata> {
        self.metadata_map.get(tool_name)
    }

    /// 获取全部工具元数据，按工具名排序返回。
    pub fn all_metadata(&self) -> Vec<&ToolMetadata> {
        let mut items = self.metadata_map.values().collect::<Vec<_>>();
        items.sort_by(|left, right| left.name.cmp(&right.name));
        items
    }
}

/// 工具调度器。
pub struct ToolDispatcher<'a> {
    /// 工具运行时配置。
    runtime_config: ToolRuntimeConfig,
    /// 工具注册中心。
    registry: ToolRegistry,
    /// 按轮次统计的执行状态。
    round_states: HashMap<String, RoundExecutionState>,
    /// 可选子智能体管理器。
    child_agent_manager: Option<ChildAgentManager<'a>>,
}

impl<'a> ToolDispatcher<'a> {
    /// 构造默认工具调度器。
    pub fn new(runtime_config: ToolRuntimeConfig) -> Self {
        Self {
            runtime_config,
            registry: ToolRegistry::with_default_tools(),
            round_states: HashMap::new(),
            child_agent_manager: None,
        }
    }

    /// 注入子智能体管理器。
    pub fn with_child_agent_manager(mut self, manager: ChildAgentManager<'a>) -> Self {
        self.child_agent_manager = Some(manager);
        self
    }

    /// 获取工具注册中心。
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// 获取运行时配置。
    pub fn runtime_config(&self) -> &ToolRuntimeConfig {
        &self.runtime_config
    }

    /// 执行一次工具调用。
    pub fn execute(
        &mut self,
        request: ToolCallRequest,
        session_mode: SessionApprovalMode,
    ) -> ToolResponse {
        let Some(metadata) = self.registry.metadata(&request.tool_name).cloned() else {
            return ToolResponse::failed(
                &request,
                true,
                "validation_error",
                "TOOL_NOT_REGISTERED",
                format!("工具未注册：{}。", request.tool_name),
                false,
            );
        };

        if let Err(message) = validate_arguments("$", &request.arguments, &metadata.schema) {
            self.record_failure(&request.tool_name, &request.round_id);
            return ToolResponse::failed(
                &request,
                metadata.visible,
                "validation_error",
                "INVALID_ARGUMENT",
                message,
                true,
            );
        }

        let current_total_calls = self
            .round_states
            .get(&request.round_id)
            .map(|state| state.total_calls)
            .unwrap_or(0);
        if current_total_calls >= self.runtime_config.max_tool_calls_per_round {
            return ToolResponse::blocked(
                &request,
                metadata.visible,
                "call_limit",
                "TOOL_CALL_LIMIT_EXCEEDED",
                format!(
                    "当前轮工具调用已达到上限 {} 次，请结束循环或改用其它策略。",
                    self.runtime_config.max_tool_calls_per_round
                ),
                false,
            );
        }

        let current_failure_streak = self
            .round_states
            .get(&request.round_id)
            .map(|state| state.failure_streak_for(&request.tool_name))
            .unwrap_or(0);
        if current_failure_streak >= self.runtime_config.tool_failure_streak_limit {
            return ToolResponse::blocked(
                &request,
                metadata.visible,
                "circuit_breaker",
                "TOOL_CIRCUIT_OPEN",
                format!(
                    "同一工具在本轮中已连续失败 {} 次，请尝试其它工具。",
                    self.runtime_config.tool_failure_streak_limit
                ),
                false,
            );
        }

        if let Err(response) = self.enforce_approval_and_boundary(&metadata, &request, session_mode)
        {
            return response;
        }

        self.round_state_mut(&request.round_id).total_calls += 1;

        let response = match metadata.executor_key.as_str() {
            "run_shell" => self.execute_run_shell(&request, &metadata),
            "read_file" => self.execute_read_file(&request, &metadata),
            "write_file" => self.execute_write_file(&request, &metadata),
            "load_skill" => self.execute_load_skill(&request, &metadata),
            "plan_tool" => self.execute_plan_tool(&request, &metadata),
            "create_agent" => self.execute_create_agent(&request, &metadata),
            _ => ToolResponse::failed(
                &request,
                metadata.visible,
                "validation_error",
                "TOOL_EXECUTOR_NOT_FOUND",
                format!("工具执行器不存在：{}。", metadata.executor_key),
                false,
            ),
        };

        match response.status {
            crate::domain::tool::ToolExecutionStatus::Success => {
                self.record_success(&request.tool_name, &request.round_id);
            }
            crate::domain::tool::ToolExecutionStatus::Failed => {
                self.record_failure(&request.tool_name, &request.round_id);
            }
            crate::domain::tool::ToolExecutionStatus::Blocked => {}
        }

        response
    }

    /// 执行 `create_agent` 工具族。
    fn execute_create_agent(
        &self,
        request: &ToolCallRequest,
        metadata: &ToolMetadata,
    ) -> ToolResponse {
        let Some(manager) = &self.child_agent_manager else {
            return ToolResponse::failed(
                request,
                metadata.visible,
                "not_implemented",
                "TOOL_NOT_IMPLEMENTED",
                "当前阶段尚未注入子智能体管理器。",
                false,
            );
        };

        let action = request.arguments["action"].as_str().unwrap_or_default();
        match action {
            "create" => {
                let task = request.arguments["task"].as_str().unwrap_or_default();
                let mode = match request.arguments["mode"].as_str().unwrap_or("inherit") {
                    "isolated" => ChildAgentMode::Isolated,
                    _ => ChildAgentMode::Inherit,
                };
                match manager.create_child_agent(CreateChildAgentRequest {
                    parent_session_id: request.session_id.clone(),
                    parent_agent_id: request.agent_id.clone(),
                    mode: mode.clone(),
                    task_summary: task.to_string(),
                    inherited_context: None,
                }) {
                    Ok(result) => ToolResponse::success(
                        request,
                        metadata.visible,
                        None,
                        "子智能体创建成功",
                        json!({
                            "child_agent_id": result.child_agent_id,
                            "child_session_id": result.child_session_id,
                            "mode": mode.as_str()
                        }),
                    ),
                    Err(error) => ToolResponse::failed(
                        request,
                        metadata.visible,
                        "execution_error",
                        "CHILD_AGENT_CREATE_FAILED",
                        error.to_string(),
                        false,
                    ),
                }
            }
            "dispatch" => {
                let child_agent_id = request.arguments["child_agent_id"]
                    .as_str()
                    .unwrap_or_default();
                let task = request.arguments["task"].as_str().unwrap_or_default();
                match manager.dispatch_child_agent(ChildAgentDispatchRequest {
                    child_agent_id: child_agent_id.to_string(),
                    task_summary: task.to_string(),
                    result_summary: Some(task.to_string()),
                }) {
                    Ok(result) => ToolResponse::success(
                        request,
                        metadata.visible,
                        None,
                        "子智能体派发成功",
                        json!({
                            "child_agent_id": result.child_agent_id,
                            "child_session_id": result.child_session_id,
                            "status": result.current_status
                        }),
                    ),
                    Err(error) => ToolResponse::failed(
                        request,
                        metadata.visible,
                        "execution_error",
                        "CHILD_AGENT_DISPATCH_FAILED",
                        error.to_string(),
                        false,
                    ),
                }
            }
            "destroy" => {
                let child_agent_id = request.arguments["child_agent_id"]
                    .as_str()
                    .unwrap_or_default();
                match manager.destroy_child_agent(child_agent_id) {
                    Ok(result) => ToolResponse::success(
                        request,
                        metadata.visible,
                        None,
                        "子智能体销毁成功",
                        json!({
                            "child_agent_id": result.child_agent_id,
                            "child_session_id": result.child_session_id,
                            "status": result.current_status
                        }),
                    ),
                    Err(error) => ToolResponse::failed(
                        request,
                        metadata.visible,
                        "execution_error",
                        "CHILD_AGENT_DESTROY_FAILED",
                        error.to_string(),
                        false,
                    ),
                }
            }
            _ => ToolResponse::failed(
                request,
                metadata.visible,
                "validation_error",
                "INVALID_ARGUMENT",
                "create_agent.action 只支持 create、dispatch、destroy。",
                true,
            ),
        }
    }

    /// 执行 `run_shell`。
    fn execute_run_shell(
        &self,
        request: &ToolCallRequest,
        metadata: &ToolMetadata,
    ) -> ToolResponse {
        let command_text = request.arguments["command"].as_str().unwrap_or_default();
        let cwd_path =
            match resolve_shell_cwd(&self.runtime_config.workspace_root_path, &request.arguments) {
                Ok(path) => path,
                Err(message) => {
                    return ToolResponse::failed(
                        request,
                        metadata.visible,
                        "validation_error",
                        "INVALID_ARGUMENT",
                        message,
                        true,
                    );
                }
            };

        let output = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(command_text)
            .current_dir(&cwd_path)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);
                if output.status.success() {
                    ToolResponse::success(
                        request,
                        metadata.visible,
                        Some(exit_code),
                        "命令执行成功",
                        json!({
                            "command": command_text,
                            "cwd": cwd_path.to_string_lossy().to_string(),
                            "stdout": stdout,
                            "stderr": stderr
                        }),
                    )
                } else {
                    ToolResponse::failed(
                        request,
                        metadata.visible,
                        "execution_error",
                        "SHELL_COMMAND_FAILED",
                        format!("PowerShell 命令执行失败，退出码：{exit_code}。"),
                        true,
                    )
                }
            }
            Err(error) => ToolResponse::failed(
                request,
                metadata.visible,
                "execution_error",
                "SHELL_LAUNCH_FAILED",
                format!("启动 PowerShell 失败：{error}"),
                true,
            ),
        }
    }

    /// 执行 `read_file`。
    fn execute_read_file(
        &self,
        request: &ToolCallRequest,
        metadata: &ToolMetadata,
    ) -> ToolResponse {
        let path_value = request.arguments["path"].as_str().unwrap_or_default();
        match resolve_file_path(&self.runtime_config.workspace_root_path, path_value) {
            Ok(path) => match fs::read_to_string(&path) {
                Ok(content) => ToolResponse::success(
                    request,
                    metadata.visible,
                    None,
                    "文件读取成功",
                    json!({
                        "path": path.to_string_lossy().to_string(),
                        "content": content
                    }),
                ),
                Err(error) => ToolResponse::failed(
                    request,
                    metadata.visible,
                    "execution_error",
                    "FILE_NOT_FOUND",
                    format!("读取文件失败：{}，原因：{error}", path.display()),
                    true,
                ),
            },
            Err(message) => ToolResponse::failed(
                request,
                metadata.visible,
                "validation_error",
                "INVALID_ARGUMENT",
                message,
                true,
            ),
        }
    }

    /// 执行 `write_file`。
    fn execute_write_file(
        &self,
        request: &ToolCallRequest,
        metadata: &ToolMetadata,
    ) -> ToolResponse {
        let path_value = request.arguments["path"].as_str().unwrap_or_default();
        let content = request.arguments["content"].as_str().unwrap_or_default();
        let mode = request.arguments["mode"].as_str().unwrap_or_default();

        let path = match resolve_file_path(&self.runtime_config.workspace_root_path, path_value) {
            Ok(path) => path,
            Err(message) => {
                return ToolResponse::failed(
                    request,
                    metadata.visible,
                    "validation_error",
                    "INVALID_ARGUMENT",
                    message,
                    true,
                );
            }
        };

        let result = match mode {
            "create" => write_file_create(&path, content),
            "replace_range" => {
                write_file_replace_range(&path, content, &request.arguments["range"])
            }
            _ => Err((
                "INVALID_ARGUMENT".to_string(),
                "write_file.mode 只支持 create 或 replace_range。".to_string(),
            )),
        };

        match result {
            Ok(()) => ToolResponse::success(
                request,
                metadata.visible,
                None,
                "文件写入成功",
                json!({
                    "path": path.to_string_lossy().to_string(),
                    "mode": mode
                }),
            ),
            Err((error_code, message)) => ToolResponse::failed(
                request,
                metadata.visible,
                "execution_error",
                &error_code,
                message,
                true,
            ),
        }
    }

    /// 执行 `load_skill`。
    fn execute_load_skill(
        &self,
        request: &ToolCallRequest,
        metadata: &ToolMetadata,
    ) -> ToolResponse {
        let skill_name = request.arguments["name"].as_str().unwrap_or_default();
        match find_skill_file(&self.runtime_config.skill_root_path, skill_name) {
            Some(path) => match fs::read_to_string(&path) {
                Ok(content) => ToolResponse::success(
                    request,
                    metadata.visible,
                    None,
                    "技能加载成功",
                    json!({
                        "name": skill_name,
                        "path": path.to_string_lossy().to_string(),
                        "content": content
                    }),
                ),
                Err(error) => ToolResponse::failed(
                    request,
                    metadata.visible,
                    "execution_error",
                    "SKILL_READ_FAILED",
                    format!("读取技能文件失败：{}，原因：{error}", path.display()),
                    true,
                ),
            },
            None => ToolResponse::failed(
                request,
                metadata.visible,
                "execution_error",
                "SKILL_NOT_FOUND",
                format!("未找到技能：{skill_name}。"),
                true,
            ),
        }
    }

    /// 执行 `plan_tool`。
    fn execute_plan_tool(
        &self,
        request: &ToolCallRequest,
        metadata: &ToolMetadata,
    ) -> ToolResponse {
        let steps = request.arguments["steps"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                json!({
                    "order": index + 1,
                    "step": value.as_str().unwrap_or_default()
                })
            })
            .collect::<Vec<_>>();

        ToolResponse::success(
            request,
            metadata.visible,
            None,
            "计划创建成功",
            json!({
                "steps": steps
            }),
        )
    }

    /// 执行审批矩阵与工作区边界检查。
    fn enforce_approval_and_boundary(
        &self,
        metadata: &ToolMetadata,
        request: &ToolCallRequest,
        session_mode: SessionApprovalMode,
    ) -> Result<(), ToolResponse> {
        if metadata.default_permission == ToolPermission::WorkspaceOnly {
            if let Err(message) = validate_workspace_boundary(
                &self.runtime_config.workspace_root_path,
                &request.tool_name,
                &request.arguments,
            ) {
                return Err(ToolResponse::blocked(
                    request,
                    metadata.visible,
                    "workspace_boundary",
                    "WORKSPACE_BOUNDARY_VIOLATION",
                    message,
                    false,
                ));
            }
        }

        match (session_mode, metadata.default_permission) {
            (SessionApprovalMode::Ask, ToolPermission::Allow)
            | (SessionApprovalMode::Ask, ToolPermission::Deny)
            | (SessionApprovalMode::Ask, ToolPermission::WorkspaceOnly) => {
                Err(ToolResponse::blocked(
                    request,
                    metadata.visible,
                    "approval_required",
                    "APPROVAL_REQUIRED",
                    "该工具需要人工确认后才能执行。",
                    false,
                ))
            }
            (SessionApprovalMode::Auto, ToolPermission::Deny) => Err(ToolResponse::blocked(
                request,
                metadata.visible,
                "policy_denied",
                "TOOL_AUTO_DENIED",
                "当前审批模式为 auto，默认拒绝工具不会自动执行。",
                false,
            )),
            _ => Ok(()),
        }
    }

    /// 获取指定轮次的执行状态。
    fn round_state_mut(&mut self, round_id: &str) -> &mut RoundExecutionState {
        self.round_states
            .entry(round_id.to_string())
            .or_insert_with(RoundExecutionState::default)
    }

    /// 记录一次成功调用。
    fn record_success(&mut self, tool_name: &str, round_id: &str) {
        self.round_state_mut(round_id)
            .failure_streak_by_tool
            .insert(tool_name.to_string(), 0);
    }

    /// 记录一次失败调用。
    fn record_failure(&mut self, tool_name: &str, round_id: &str) {
        let round_state = self.round_state_mut(round_id);
        let streak = round_state
            .failure_streak_by_tool
            .entry(tool_name.to_string())
            .or_insert(0);
        *streak += 1;
    }
}

/// 单轮执行状态。
#[derive(Debug, Default, Clone)]
struct RoundExecutionState {
    /// 当前轮累计调用次数。
    total_calls: u32,
    /// 每个工具的连续失败次数。
    failure_streak_by_tool: HashMap<String, u32>,
}

impl RoundExecutionState {
    /// 获取指定工具的连续失败次数。
    fn failure_streak_for(&self, tool_name: &str) -> u32 {
        self.failure_streak_by_tool
            .get(tool_name)
            .copied()
            .unwrap_or(0)
    }
}

/// 校验参数值是否符合模式定义。
fn validate_arguments(path: &str, value: &Value, schema: &ToolSchemaNode) -> Result<(), String> {
    match schema.value_type {
        ToolValueType::String => {
            if value.is_string() {
                Ok(())
            } else {
                Err(format!("参数 {} 缺失或类型错误。", display_path(path)))
            }
        }
        ToolValueType::Integer => {
            if value.is_i64() || value.is_u64() {
                Ok(())
            } else {
                Err(format!("参数 {} 缺失或类型错误。", display_path(path)))
            }
        }
        ToolValueType::Boolean => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(format!("参数 {} 缺失或类型错误。", display_path(path)))
            }
        }
        ToolValueType::Object => {
            let Some(object) = value.as_object() else {
                return Err(format!("参数 {} 缺失或类型错误。", display_path(path)));
            };

            for required_key in &schema.required {
                let Some(required_value) = object.get(required_key) else {
                    return Err(format!("参数 {} 缺失或类型错误。", required_key));
                };

                if let Some(required_schema) = schema.properties.get(required_key) {
                    validate_arguments(required_key, required_value, required_schema)?;
                }
            }

            for (key, property_schema) in &schema.properties {
                if let Some(property_value) = object.get(key) {
                    validate_arguments(key, property_value, property_schema)?;
                }
            }

            Ok(())
        }
        ToolValueType::Array => {
            let Some(items) = value.as_array() else {
                return Err(format!("参数 {} 缺失或类型错误。", display_path(path)));
            };
            let Some(item_schema) = &schema.items else {
                return Ok(());
            };

            for item in items {
                validate_arguments(path, item, item_schema)?;
            }
            Ok(())
        }
    }
}

/// 把内部路径标识转换成用户可见参数名。
fn display_path(path: &str) -> &str {
    if path == "$" { "arguments" } else { path }
}

/// 校验工作区边界。
fn validate_workspace_boundary(
    workspace_root_path: &Path,
    tool_name: &str,
    arguments: &Value,
) -> Result<(), String> {
    match tool_name {
        "run_shell" => {
            let cwd = resolve_shell_cwd(workspace_root_path, arguments)?;
            if is_path_within_workspace(workspace_root_path, &cwd) {
                Ok(())
            } else {
                Err(format!(
                    "run_shell.cwd 超出工作区范围：{}",
                    cwd.to_string_lossy()
                ))
            }
        }
        "read_file" | "write_file" => {
            let path_value = arguments["path"]
                .as_str()
                .ok_or_else(|| "参数 path 缺失或类型错误。".to_string())?;
            let path = resolve_file_path(workspace_root_path, path_value)?;
            if is_path_within_workspace(workspace_root_path, &path) {
                Ok(())
            } else {
                Err(format!(
                    "文件路径超出工作区范围：{}",
                    path.to_string_lossy()
                ))
            }
        }
        _ => Ok(()),
    }
}

/// 解析 `run_shell` 的工作目录。
fn resolve_shell_cwd(workspace_root_path: &Path, arguments: &Value) -> Result<PathBuf, String> {
    match arguments.get("cwd").and_then(Value::as_str) {
        Some(cwd) => normalize_path(workspace_root_path, cwd),
        None => Ok(normalize_existing_or_self(workspace_root_path)),
    }
}

/// 解析文件路径。
fn resolve_file_path(workspace_root_path: &Path, raw_path: &str) -> Result<PathBuf, String> {
    normalize_path(workspace_root_path, raw_path)
}

/// 归一化路径。
fn normalize_path(workspace_root_path: &Path, raw_path: &str) -> Result<PathBuf, String> {
    if raw_path.trim().is_empty() {
        return Err("路径参数不能为空。".to_string());
    }

    let input_path = PathBuf::from(raw_path);
    let absolute_path = if input_path.is_absolute() {
        input_path
    } else {
        workspace_root_path.join(input_path)
    };

    Ok(normalize_existing_or_self(&absolute_path))
}

/// 如果路径已存在则使用规范路径，否则使用绝对路径本身。
fn normalize_existing_or_self(path: &Path) -> PathBuf {
    let normalized = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_windows_verbatim_prefix(&normalized)
}

/// 判断路径是否位于工作区内。
fn is_path_within_workspace(workspace_root_path: &Path, target_path: &Path) -> bool {
    let normalized_workspace = normalize_existing_or_self(workspace_root_path);
    let normalized_target = normalize_existing_or_self(target_path);
    normalized_target.starts_with(&normalized_workspace)
}

/// 去掉 Windows 规范路径前缀，避免同一路径因 `\\?\` 前缀不同而比较失败。
fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(stripped) = path_text.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// 执行 `write_file.create`。
fn write_file_create(path: &Path, content: &str) -> Result<(), (String, String)> {
    if let Some(parent_directory) = path.parent() {
        fs::create_dir_all(parent_directory).map_err(|error| {
            (
                "FILE_WRITE_FAILED".to_string(),
                format!(
                    "创建目录失败：{}，原因：{error}",
                    parent_directory.display()
                ),
            )
        })?;
    }

    fs::write(path, content).map_err(|error| {
        (
            "FILE_WRITE_FAILED".to_string(),
            format!("写入文件失败：{}，原因：{error}", path.display()),
        )
    })
}

/// 执行 `write_file.replace_range`。
fn write_file_replace_range(
    path: &Path,
    content: &str,
    range_value: &Value,
) -> Result<(), (String, String)> {
    if !path.exists() {
        return Err((
            "FILE_NOT_FOUND".to_string(),
            format!("目标文件不存在：{}。", path.display()),
        ));
    }

    let file_content = fs::read_to_string(path).map_err(|error| {
        (
            "FILE_READ_FAILED".to_string(),
            format!("读取待替换文件失败：{}，原因：{error}", path.display()),
        )
    })?;

    let start_line = range_value["start_line"].as_i64().ok_or_else(|| {
        (
            "INVALID_ARGUMENT".to_string(),
            "replace_range.start_line 缺失或类型错误。".to_string(),
        )
    })?;
    let end_line = range_value["end_line"].as_i64().ok_or_else(|| {
        (
            "INVALID_ARGUMENT".to_string(),
            "replace_range.end_line 缺失或类型错误。".to_string(),
        )
    })?;

    let mut lines = file_content
        .lines()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if start_line <= 0 || end_line < start_line || end_line as usize > lines.len() {
        return Err((
            "RANGE_OUT_OF_BOUNDS".to_string(),
            format!(
                "替换范围超出文件边界：start_line={}，end_line={}。",
                start_line, end_line
            ),
        ));
    }

    let replacement_lines = if content.is_empty() {
        vec![String::new()]
    } else {
        content
            .split('\n')
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    lines.splice(
        (start_line as usize - 1)..=(end_line as usize - 1),
        replacement_lines,
    );

    let mut output = lines.join("\n");
    if file_content.ends_with('\n') || output.is_empty() {
        output.push('\n');
    }

    fs::write(path, output).map_err(|error| {
        (
            "FILE_WRITE_FAILED".to_string(),
            format!("写回替换结果失败：{}，原因：{error}", path.display()),
        )
    })
}

/// 查找技能文件。
fn find_skill_file(skill_root_path: &Path, skill_name: &str) -> Option<PathBuf> {
    let direct_path = skill_root_path.join(skill_name).join("SKILL.md");
    if direct_path.exists() {
        return Some(direct_path);
    }

    recursive_find_skill(skill_root_path, skill_name)
}

/// 递归查找技能目录。
fn recursive_find_skill(root: &Path, skill_name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some(skill_name) {
                let file = path.join("SKILL.md");
                if file.exists() {
                    return Some(file);
                }
            }
            if let Some(found) = recursive_find_skill(&path, skill_name) {
                return Some(found);
            }
        }
    }

    None
}

/// 默认 `run_shell` 元数据。
fn default_run_shell_metadata() -> ToolMetadata {
    ToolMetadata {
        name: "run_shell".to_string(),
        description: "在 Windows PowerShell 中执行命令".to_string(),
        schema: ToolSchemaNode::object(
            vec!["command"],
            BTreeMap::from([
                ("command".to_string(), ToolSchemaNode::string()),
                ("cwd".to_string(), ToolSchemaNode::string()),
            ]),
        ),
        default_permission: ToolPermission::WorkspaceOnly,
        visible: true,
        background: false,
        executor_key: "run_shell".to_string(),
    }
}

/// 默认 `read_file` 元数据。
fn default_read_file_metadata() -> ToolMetadata {
    ToolMetadata {
        name: "read_file".to_string(),
        description: "读取工作区内文件内容".to_string(),
        schema: ToolSchemaNode::object(
            vec!["path"],
            BTreeMap::from([("path".to_string(), ToolSchemaNode::string())]),
        ),
        default_permission: ToolPermission::WorkspaceOnly,
        visible: true,
        background: false,
        executor_key: "read_file".to_string(),
    }
}

/// 默认 `write_file` 元数据。
fn default_write_file_metadata() -> ToolMetadata {
    ToolMetadata {
        name: "write_file".to_string(),
        description: "在工作区内创建文件或按范围替换内容".to_string(),
        schema: ToolSchemaNode::object(
            vec!["path", "content", "mode"],
            BTreeMap::from([
                ("path".to_string(), ToolSchemaNode::string()),
                ("content".to_string(), ToolSchemaNode::string()),
                ("mode".to_string(), ToolSchemaNode::string()),
                (
                    "range".to_string(),
                    ToolSchemaNode::object(
                        vec!["start_line", "end_line"],
                        BTreeMap::from([
                            ("start_line".to_string(), ToolSchemaNode::integer()),
                            ("end_line".to_string(), ToolSchemaNode::integer()),
                        ]),
                    ),
                ),
            ]),
        ),
        default_permission: ToolPermission::WorkspaceOnly,
        visible: true,
        background: false,
        executor_key: "write_file".to_string(),
    }
}

/// 默认 `load_skill` 元数据。
fn default_load_skill_metadata() -> ToolMetadata {
    ToolMetadata {
        name: "load_skill".to_string(),
        description: "读取指定技能的完整内容".to_string(),
        schema: ToolSchemaNode::object(
            vec!["name"],
            BTreeMap::from([("name".to_string(), ToolSchemaNode::string())]),
        ),
        default_permission: ToolPermission::Allow,
        visible: true,
        background: false,
        executor_key: "load_skill".to_string(),
    }
}

/// 默认 `plan_tool` 元数据。
fn default_plan_tool_metadata() -> ToolMetadata {
    ToolMetadata {
        name: "plan_tool".to_string(),
        description: "创建有序计划步骤".to_string(),
        schema: ToolSchemaNode::object(
            vec!["steps"],
            BTreeMap::from([(
                "steps".to_string(),
                ToolSchemaNode::array(ToolSchemaNode::string()),
            )]),
        ),
        default_permission: ToolPermission::Deny,
        visible: true,
        background: false,
        executor_key: "plan_tool".to_string(),
    }
}

/// 默认 `create_agent` 元数据。
fn default_create_agent_metadata() -> ToolMetadata {
    ToolMetadata {
        name: "create_agent".to_string(),
        description: "创建、继续派发或销毁子智能体".to_string(),
        schema: ToolSchemaNode::object(
            vec!["action"],
            BTreeMap::from([
                ("action".to_string(), ToolSchemaNode::string()),
                ("mode".to_string(), ToolSchemaNode::string()),
                ("task".to_string(), ToolSchemaNode::string()),
                ("child_agent_id".to_string(), ToolSchemaNode::string()),
            ]),
        ),
        default_permission: ToolPermission::Deny,
        visible: true,
        background: false,
        executor_key: "create_agent".to_string(),
    }
}
