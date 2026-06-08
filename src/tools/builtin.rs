//! 内置工具实现。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::agent::manager::SubagentManager;
use crate::domain::ReplaceRange;
use crate::ipc::bus::EventBus;
use crate::mcp::client::McpClientManager;
use crate::session::snapshot::read_session_ini;
use crate::tools::registry::{ToolExecutionContext, ToolHandler};
use crate::utils::fs::{read_optional_utf8, replace_file_range, write_utf8};

#[derive(Deserialize)]
struct ReadFileInput {
    path: String,
}

#[derive(Deserialize)]
struct WriteFileInput {
    path: String,
    content: Option<String>,
    replace_range: Option<ReplaceRange>,
}

#[derive(Deserialize)]
struct EditFileInput {
    path: String,
    old_text: String,
    new_text: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct ListDirInput {
    path: Option<String>,
    include_hidden: Option<bool>,
}

#[derive(Deserialize)]
struct FileSearchInput {
    query: String,
    root_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct GrepFilesInput {
    pattern: String,
    root_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct DiagnosticsInput {
    root_path: Option<String>,
}

#[derive(Deserialize)]
struct ReviewInput {
    root_path: Option<String>,
}

#[derive(Deserialize)]
struct ProjectMapInput {
    root_path: Option<String>,
    max_depth: Option<usize>,
    max_entries: Option<usize>,
}

#[derive(Deserialize)]
struct ValidateDataInput {
    data: Value,
    required_fields: Vec<String>,
}

#[derive(Deserialize)]
struct RunTestsInput {
    command: Option<String>,
    working_directory: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct NoteInput {
    title: Option<String>,
    content: String,
}

#[derive(Deserialize)]
struct RememberInput {
    key: Option<String>,
    content: String,
}

#[derive(Deserialize)]
struct RecallArchiveInput {
    query: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct NotifyInput {
    message: String,
}

#[derive(Deserialize)]
struct FimEditInput {
    path: String,
    old_text: String,
    new_text: String,
}

#[derive(Deserialize, Serialize)]
struct RequestUserInputOption {
    label: String,
    description: String,
}

#[derive(Deserialize, Serialize)]
struct RequestUserInputQuestion {
    header: String,
    id: String,
    question: String,
    options: Vec<RequestUserInputOption>,
}

#[derive(Deserialize)]
struct RequestUserInputInput {
    questions: Vec<RequestUserInputQuestion>,
}

#[derive(Deserialize)]
struct GitStatusInput {
    root_path: Option<String>,
}

#[derive(Deserialize)]
struct GitDiffInput {
    root_path: Option<String>,
    revision: Option<String>,
}

#[derive(Deserialize)]
struct GitLogInput {
    root_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct GitShowInput {
    root_path: Option<String>,
    object: String,
}

#[derive(Deserialize)]
struct GitBlameInput {
    root_path: Option<String>,
    path: String,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

#[derive(Deserialize)]
struct ExecShellInput {
    command: String,
    working_directory: Option<String>,
}

#[derive(Deserialize)]
struct ExecShellWaitInput {
    process_id: String,
    idle_timeout_ms: Option<u64>,
    max_lines: Option<usize>,
}

#[derive(Deserialize)]
struct ExecShellInteractInput {
    process_id: String,
    input: String,
}

#[derive(Deserialize)]
struct ExecShellCancelInput {
    process_id: String,
}

#[derive(Deserialize)]
struct RunShellInput {
    command: String,
    working_directory: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct RlmOpenInput {
    working_directory: Option<String>,
}

#[derive(Deserialize)]
struct RlmEvalInput {
    process_id: String,
    command: String,
}

#[derive(Deserialize)]
struct RlmConfigureInput {
    process_id: String,
    working_directory: Option<String>,
}

#[derive(Deserialize)]
struct RlmCloseInput {
    process_id: String,
}

#[derive(Deserialize)]
struct LoadSkillInput {
    identifier: String,
}

#[derive(Deserialize)]
struct ReadToolResultInput {
    handle: String,
}

#[derive(Deserialize)]
struct RetrieveToolResultInput {
    tool_call_id: String,
    mode: Option<String>,
    start_char: Option<usize>,
    length_chars: Option<usize>,
    keyword: Option<String>,
    context_chars: Option<usize>,
}

#[derive(Deserialize)]
struct HandleReadInput {
    handle: String,
    max_chars: Option<usize>,
}

#[derive(Deserialize)]
struct FinanceInput {
    symbol: String,
}

#[derive(Deserialize)]
struct FetchUrlInput {
    url: String,
}

#[derive(Deserialize)]
struct GithubGetInput {
    endpoint: String,
}

#[derive(Deserialize)]
struct WebSearchInput {
    query: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct WebRunInput {
    steps: Vec<WebRunStep>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WebRunStep {
    Open { url: String },
    Click { text_contains: String },
    Find { pattern: String },
    ExtractText { selector: Option<String> },
}

#[derive(Deserialize)]
struct ConnectMcpInput {
    server_id: String,
}

#[derive(Deserialize)]
struct CallMcpToolInput {
    server_id: String,
    tool_name: String,
    arguments: Value,
}

#[derive(Deserialize)]
struct AgentOpenInput {
    mode: String,
    inherit_context: Option<bool>,
    allowed_paths: Option<Vec<String>>,
    task_spec: Value,
    parent_agent_id: Option<String>,
}

#[derive(Deserialize)]
struct AgentEvalInput {
    agent_id: String,
    input: Value,
}

#[derive(Deserialize)]
struct AgentCloseInput {
    agent_id: String,
}

#[derive(Deserialize)]
struct PlanWriteInput {
    plan_type: String,
    content: String,
}

#[derive(Deserialize)]
struct TaskCreateInput {
    name: String,
    command: String,
    validation_command: Option<String>,
}

#[derive(Deserialize)]
struct TaskRunInput {
    task_id: String,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct TaskShellStartInput {
    task_id: String,
    working_directory: Option<String>,
}

#[derive(Deserialize)]
struct TaskShellWaitInput {
    task_id: String,
    idle_timeout_ms: Option<u64>,
    max_lines: Option<usize>,
}

#[derive(Deserialize)]
struct AutomationCreateInput {
    name: String,
    kind: String,
    status: String,
    definition: Value,
}

#[derive(Deserialize)]
struct AutomationRunOnceInput {
    automation_id: String,
    timeout_ms: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct TaskRecord {
    id: String,
    name: String,
    status: String,
    command: String,
    validation_command: Option<String>,
    bound_process_id: Option<String>,
    created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct AutomationRecord {
    id: String,
    name: String,
    kind: String,
    status: String,
    definition_json: String,
    last_run_at: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SearchResultItem {
    title: String,
    link: String,
    snippet: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FinanceQuoteItem {
    symbol: String,
    currency: String,
    regular_market_price: f64,
    previous_close: Option<f64>,
    timestamp: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct RlmProcessRecord {
    id: String,
    shell_program: String,
    working_directory: String,
    status: String,
    created_at: String,
    updated_at: String,
}

struct ManagedRlmProcess {
    working_directory: PathBuf,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ExecProcessRecord {
    id: String,
    shell_program: String,
    working_directory: String,
    status: String,
    created_at: String,
    updated_at: String,
}

struct ManagedExecProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(serde::Serialize)]
struct WebRunStepResult {
    step: String,
    url: String,
    success: bool,
    output: String,
}

#[derive(serde::Serialize)]
struct DirectoryEntryItem {
    name: String,
    path: String,
    is_dir: bool,
}

#[derive(serde::Serialize)]
struct FileSearchItem {
    name: String,
    path: String,
}

#[derive(serde::Serialize)]
struct GrepMatchItem {
    path: String,
    line_no: usize,
    line: String,
}

#[derive(serde::Serialize)]
struct ProjectMapItem {
    path: String,
    is_dir: bool,
    depth: usize,
}

/// 读取文件工具。
pub struct ReadFileTool;
/// 写入文件工具。
pub struct WriteFileTool;
/// Shell 执行工具。
pub struct RunShellTool;
/// RLM 打开工具。
pub struct RlmOpenTool;
/// RLM 求值工具。
pub struct RlmEvalTool;
/// RLM 配置工具。
pub struct RlmConfigureTool;
/// RLM 关闭工具。
pub struct RlmCloseTool;
/// Skill 加载工具。
pub struct LoadSkillTool;
/// 工具结果读取工具。
pub struct ReadToolResultTool;
/// URL 抓取工具。
pub struct FetchUrlTool;
/// GitHub 只读接口工具。
pub struct GithubGetTool;
/// 网络搜索工具。
pub struct WebSearchTool;
/// 网页步骤执行工具。
pub struct WebRunTool;
/// MCP 发现工具。
pub struct DiscoverMcpServersTool;
/// MCP 连接工具。
pub struct ConnectMcpServerTool;
/// MCP 调用工具。
pub struct CallMcpTool;
/// 子 Agent 创建工具。
pub struct AgentOpenTool;
/// 子 Agent 执行工具。
pub struct AgentEvalTool;
/// 子 Agent 关闭工具。
pub struct AgentCloseTool;
/// 计划写入工具。
pub struct PlanWriteTool;
/// 任务创建工具。
pub struct TaskCreateTool;
/// 任务执行工具。
pub struct TaskRunTool;
/// 自动化创建工具。
pub struct AutomationCreateTool;
/// 自动化单次执行工具。
pub struct AutomationRunOnceTool;

#[async_trait]
impl ToolHandler for ReadFileTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ReadFileInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("read_file 参数非法：{}", error))?;
        let path = resolve_path(&context.workspace_root, &input.path);
        let content = read_optional_utf8(&path)?
            .ok_or_else(|| anyhow!("目标文件不存在：{}", path.display()))?;
        Ok(content)
    }
}

#[async_trait]
impl ToolHandler for WriteFileTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: WriteFileInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("write_file 参数非法：{}", error))?;
        let path = resolve_path(&context.workspace_root, &input.path);

        if input.content.is_some() && input.replace_range.is_some() {
            return Err(anyhow!(
                "write_file 同时提供了 content 与 replace_range，存在冲突，请二选一"
            ));
        }

        match (input.content, input.replace_range) {
            (Some(content), None) => {
                write_utf8(&path, &content)?;
                Ok(format!("已写入文件：{}", path.display()))
            }
            (None, Some(replace_range)) => {
                replace_file_range(
                    &path,
                    replace_range.start_line,
                    replace_range.end_line,
                    &replace_range.new_content,
                )?;
                Ok(format!(
                    "已按行替换文件：{}（{}-{}）",
                    path.display(),
                    replace_range.start_line,
                    replace_range.end_line
                ))
            }
            (None, None) => Err(anyhow!("write_file 缺少 content 或 replace_range")),
            (Some(_), Some(_)) => unreachable!("前置冲突检查已覆盖该分支"),
        }
    }
}

pub struct EditFileTool;
pub struct ListDirTool;
pub struct FileSearchTool;
pub struct GrepFilesTool;
pub struct DiagnosticsTool;
pub struct ReviewTool;
pub struct ProjectMapTool;
pub struct ValidateDataTool;
pub struct RunTestsTool;
pub struct NoteTool;
pub struct RememberTool;
pub struct RecallArchiveTool;
pub struct NotifyTool;
pub struct FimEditTool;
pub struct GitStatusTool;
pub struct GitDiffTool;
pub struct GitLogTool;
pub struct GitShowTool;
pub struct GitBlameTool;
pub struct ExecShellTool;
pub struct ExecShellWaitTool;
pub struct ExecShellInteractTool;
pub struct ExecShellCancelTool;
pub struct RequestUserInputTool;

#[async_trait]
impl ToolHandler for EditFileTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: EditFileInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("edit_file 参数非法：{}", error))?;
        let path = resolve_path(&context.workspace_root, &input.path);
        let original = read_optional_utf8(&path)?
            .ok_or_else(|| anyhow!("目标文件不存在：{}", path.display()))?;

        if input.old_text.is_empty() {
            return Err(anyhow!("edit_file 的 old_text 不能为空"));
        }
        if !original.contains(&input.old_text) {
            return Err(anyhow!("未在目标文件中找到待替换内容：{}", path.display()));
        }

        let replaced = if input.replace_all.unwrap_or(false) {
            original.replace(&input.old_text, &input.new_text)
        } else {
            original.replacen(&input.old_text, &input.new_text, 1)
        };
        write_utf8(&path, &replaced)?;
        Ok(format!("已更新文件：{}", path.display()))
    }
}

#[async_trait]
impl ToolHandler for ListDirTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ListDirInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("list_dir 参数非法：{}", error))?;
        let target = input
            .path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let include_hidden = input.include_hidden.unwrap_or(false);
        let entries = std::fs::read_dir(&target)
            .with_context(|| format!("读取目录失败：{}", target.display()))?;

        let mut items = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            let metadata = entry.metadata()?;
            items.push(DirectoryEntryItem {
                name,
                path: entry.path().to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
            });
        }

        items.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(serde_json::to_string_pretty(&items)?)
    }
}

#[async_trait]
impl ToolHandler for FileSearchTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: FileSearchInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("file_search 参数非法：{}", error))?;
        if input.query.trim().is_empty() {
            return Err(anyhow!("file_search 的 query 不能为空"));
        }

        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let query = input.query.to_ascii_lowercase();
        let limit = input.limit.unwrap_or(50);
        let mut items = Vec::new();

        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.to_ascii_lowercase().contains(&query) {
                items.push(FileSearchItem {
                    name,
                    path: entry.path().to_string_lossy().to_string(),
                });
                if items.len() >= limit {
                    break;
                }
            }
        }

        Ok(serde_json::to_string_pretty(&items)?)
    }
}

#[async_trait]
impl ToolHandler for GrepFilesTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: GrepFilesInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("grep_files 参数非法：{}", error))?;
        let regex = Regex::new(&input.pattern)
            .map_err(|error| anyhow!("grep_files 正则非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let limit = input.limit.unwrap_or(100);
        let mut matches = Vec::new();

        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(content) = read_optional_utf8(entry.path())? else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(GrepMatchItem {
                        path: entry.path().to_string_lossy().to_string(),
                        line_no: index + 1,
                        line: line.to_string(),
                    });
                    if matches.len() >= limit {
                        return Ok(serde_json::to_string_pretty(&matches)?);
                    }
                }
            }
        }

        Ok(serde_json::to_string_pretty(&matches)?)
    }
}

#[async_trait]
impl ToolHandler for DiagnosticsTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: DiagnosticsInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("diagnostics 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let git_status = run_shell_command(
            &context.shell_program,
            "git status --short --branch",
            &root,
            10_000,
        )
        .await
        .unwrap_or_else(|error| format!("Git 状态不可用：{}", error));

        Ok(serde_json::to_string_pretty(&json!({
            "workspace_root": root.to_string_lossy(),
            "session_dir": context.session_dir.to_string_lossy(),
            "shell_program": context.shell_program,
            "git_status": git_status
        }))?)
    }
}

#[async_trait]
impl ToolHandler for ReviewTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ReviewInput =
            serde_json::from_value(input).map_err(|error| anyhow!("review 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());

        let status =
            run_shell_command(&context.shell_program, "git status --short", &root, 10_000).await?;
        let diff =
            run_shell_command(&context.shell_program, "git diff --stat", &root, 10_000).await?;
        Ok(format!(
            "Git 状态摘要：\n{}\n\nGit 变更统计：\n{}",
            status, diff
        ))
    }
}

#[async_trait]
impl ToolHandler for ProjectMapTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ProjectMapInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("project_map 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let max_depth = input.max_depth.unwrap_or(3);
        let max_entries = input.max_entries.unwrap_or(200);
        let mut items = Vec::new();

        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let depth = entry.depth();
            if depth == 0 || depth > max_depth {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .to_string();
            items.push(ProjectMapItem {
                path: relative,
                is_dir: entry.file_type().is_dir(),
                depth,
            });
            if items.len() >= max_entries {
                break;
            }
        }

        Ok(serde_json::to_string_pretty(&items)?)
    }
}

#[async_trait]
impl ToolHandler for ValidateDataTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: ValidateDataInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("validate_data 参数非法：{}", error))?;
        let object = input
            .data
            .as_object()
            .ok_or_else(|| anyhow!("validate_data 的 data 必须是 JSON 对象"))?;
        let missing_fields = input
            .required_fields
            .into_iter()
            .filter(|field| {
                !object.contains_key(field) || object.get(field).is_some_and(Value::is_null)
            })
            .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&json!({
            "valid": missing_fields.is_empty(),
            "missing_fields": missing_fields
        }))?)
    }
}

#[async_trait]
impl ToolHandler for RunTestsTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RunTestsInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("run_tests 参数非法：{}", error))?;
        let command = input.command.unwrap_or_else(|| "cargo test".to_string());
        let working_directory = input
            .working_directory
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        run_shell_command(
            &context.shell_program,
            &command,
            &working_directory,
            input.timeout_ms.unwrap_or(120_000),
        )
        .await
    }
}

#[async_trait]
impl ToolHandler for NoteTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: NoteInput =
            serde_json::from_value(input).map_err(|error| anyhow!("note 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("memory")
            .join("notes.json");
        let mut items: Vec<Value> = load_json_array(&path)?;
        items.push(json!({
            "id": Uuid::new_v4().to_string(),
            "title": input.title,
            "content": input.content,
            "created_at": crate::utils::time::now_rfc3339()
        }));
        write_utf8(&path, &serde_json::to_string_pretty(&items)?)?;
        Ok("已记录一次性备注。".to_string())
    }
}

#[async_trait]
impl ToolHandler for RememberTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RememberInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("remember 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("memory")
            .join("archive.json");
        let mut items: Vec<Value> = load_json_array(&path)?;
        items.push(json!({
            "id": Uuid::new_v4().to_string(),
            "kind": "memory",
            "key": input.key,
            "content": input.content,
            "created_at": crate::utils::time::now_rfc3339()
        }));
        write_utf8(&path, &serde_json::to_string_pretty(&items)?)?;
        Ok("已写入长期记忆。".to_string())
    }
}

#[async_trait]
impl ToolHandler for RecallArchiveTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RecallArchiveInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("recall_archive 参数非法：{}", error))?;
        if input.query.trim().is_empty() {
            return Err(anyhow!("recall_archive 的 query 不能为空"));
        }

        let notes_path = context
            .session_dir
            .join(".tools")
            .join("memory")
            .join("notes.json");
        let archive_path = context
            .session_dir
            .join(".tools")
            .join("memory")
            .join("archive.json");
        let mut items: Vec<Value> = load_json_array(&notes_path)?;
        items.extend(load_json_array::<Value>(&archive_path)?);
        let query = input.query.to_ascii_lowercase();
        let limit = input.limit.unwrap_or(20);
        let matched = items
            .into_iter()
            .filter(|item| item.to_string().to_ascii_lowercase().contains(&query))
            .take(limit)
            .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&matched)?)
    }
}

#[async_trait]
impl ToolHandler for NotifyTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: NotifyInput =
            serde_json::from_value(input).map_err(|error| anyhow!("notify 参数非法：{}", error))?;
        println!("通知：{}", input.message);
        Ok("已发送终端通知。".to_string())
    }
}

#[async_trait]
impl ToolHandler for FimEditTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: FimEditInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("fim_edit 参数非法：{}", error))?;
        let path = resolve_path(&context.workspace_root, &input.path);
        let original = read_optional_utf8(&path)?
            .ok_or_else(|| anyhow!("目标文件不存在：{}", path.display()))?;
        if input.old_text.is_empty() {
            return Err(anyhow!("fim_edit 的 old_text 不能为空"));
        }
        if !original.contains(&input.old_text) {
            return Err(anyhow!("未在目标文件中找到待替换内容：{}", path.display()));
        }
        let replaced = original.replacen(&input.old_text, &input.new_text, 1);
        write_utf8(&path, &replaced)?;
        Ok(format!("已完成 FIM 替换编辑：{}", path.display()))
    }
}

#[async_trait]
impl ToolHandler for GitStatusTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: GitStatusInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("git_status 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        run_shell_command(
            &context.shell_program,
            "git status --short --branch",
            &root,
            10_000,
        )
        .await
    }
}

#[async_trait]
impl ToolHandler for GitDiffTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: GitDiffInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("git_diff 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let command = if let Some(revision) = input.revision {
            format!("git diff {}", revision)
        } else {
            "git diff".to_string()
        };
        run_shell_command(&context.shell_program, &command, &root, 10_000).await
    }
}

#[async_trait]
impl ToolHandler for GitLogTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: GitLogInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("git_log 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let limit = input.limit.unwrap_or(10);
        let command = format!("git log -n {} --oneline", limit);
        run_shell_command(&context.shell_program, &command, &root, 10_000).await
    }
}

#[async_trait]
impl ToolHandler for GitShowTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: GitShowInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("git_show 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let command = format!("git show --stat {}", input.object);
        run_shell_command(&context.shell_program, &command, &root, 10_000).await
    }
}

#[async_trait]
impl ToolHandler for GitBlameTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: GitBlameInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("git_blame 参数非法：{}", error))?;
        let root = input
            .root_path
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());
        let relative_path = Path::new(&input.path);
        let line_clause = match (input.start_line, input.end_line) {
            (Some(start), Some(end)) => format!("-L {},{} ", start, end),
            (Some(start), None) => format!("-L {},{} ", start, start),
            _ => String::new(),
        };
        let command = format!("git blame {}-- {}", line_clause, relative_path.display());
        run_shell_command(&context.shell_program, &command, &root, 10_000).await
    }
}

#[async_trait]
impl ToolHandler for ExecShellTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ExecShellInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("exec_shell 参数非法：{}", error))?;
        let working_directory = input
            .working_directory
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());

        let mut command = Command::new(&context.shell_program);
        command
            .arg("-NoProfile")
            .arg("-NoLogo")
            .arg("-Command")
            .arg("-")
            .current_dir(&working_directory)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().context("启动后台 Shell 进程失败")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("无法获取后台 Shell stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("无法获取后台 Shell stdout"))?;
        let stdout = BufReader::new(stdout);

        let process_id = Uuid::new_v4().to_string();
        let process = ManagedExecProcess {
            child,
            stdin,
            stdout,
        };
        exec_registry()
            .lock()
            .await
            .insert(process_id.clone(), process);

        {
            let mut registry = exec_registry().lock().await;
            let process = registry
                .get_mut(&process_id)
                .ok_or_else(|| anyhow!("后台 Shell 进程注册失败：{}", process_id))?;
            process
                .stdin
                .write_all(format!("{} 2>&1\n", input.command).as_bytes())
                .await
                .context("写入后台 Shell 初始命令失败")?;
            process
                .stdin
                .flush()
                .await
                .context("刷新后台 Shell stdin 失败")?;
        }

        persist_exec_record(
            &context.session_dir,
            ExecProcessRecord {
                id: process_id.clone(),
                shell_program: context.shell_program.clone(),
                working_directory: working_directory.to_string_lossy().to_string(),
                status: "running".to_string(),
                created_at: crate::utils::time::now_rfc3339(),
                updated_at: crate::utils::time::now_rfc3339(),
            },
        )?;
        Ok(serde_json::json!({
            "process_id": process_id,
            "working_directory": working_directory,
            "status": "running"
        })
        .to_string())
    }
}

#[async_trait]
impl ToolHandler for ExecShellWaitTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ExecShellWaitInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("exec_shell_wait 参数非法：{}", error))?;
        let idle_timeout_ms = input.idle_timeout_ms.unwrap_or(200);
        let max_lines = input.max_lines.unwrap_or(200);

        let mut registry = exec_registry().lock().await;
        let process = registry
            .get_mut(&input.process_id)
            .ok_or_else(|| anyhow!("未找到后台 Shell 进程：{}", input.process_id))?;

        let mut output = String::new();
        let mut lines = 0usize;
        loop {
            let mut line = String::new();
            let read_future = process.stdout.read_line(&mut line);
            match timeout(Duration::from_millis(idle_timeout_ms), read_future).await {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => {
                    output.push_str(&line);
                    lines += 1;
                    if lines >= max_lines {
                        break;
                    }
                }
                Ok(Err(error)) => return Err(anyhow!("读取后台 Shell 输出失败：{}", error)),
                Err(_) => break,
            }
        }

        touch_exec_record(
            &context.session_dir,
            &input.process_id,
            None,
            Some("running"),
        )?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl ToolHandler for ExecShellInteractTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: ExecShellInteractInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("exec_shell_interact 参数非法：{}", error))?;
        let mut registry = exec_registry().lock().await;
        let process = registry
            .get_mut(&input.process_id)
            .ok_or_else(|| anyhow!("未找到后台 Shell 进程：{}", input.process_id))?;
        process
            .stdin
            .write_all(format!("{} 2>&1\n", input.input).as_bytes())
            .await
            .context("写入后台 Shell 输入失败")?;
        process
            .stdin
            .flush()
            .await
            .context("刷新后台 Shell stdin 失败")?;
        Ok("已写入后台 Shell 输入。".to_string())
    }
}

#[async_trait]
impl ToolHandler for ExecShellCancelTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ExecShellCancelInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("exec_shell_cancel 参数非法：{}", error))?;
        let mut registry = exec_registry().lock().await;
        let mut process = registry
            .remove(&input.process_id)
            .ok_or_else(|| anyhow!("未找到后台 Shell 进程：{}", input.process_id))?;
        process
            .child
            .kill()
            .await
            .context("终止后台 Shell 进程失败")?;
        touch_exec_record(
            &context.session_dir,
            &input.process_id,
            None,
            Some("cancelled"),
        )?;
        Ok(serde_json::json!({
            "process_id": input.process_id,
            "status": "cancelled"
        })
        .to_string())
    }
}

#[async_trait]
impl ToolHandler for RequestUserInputTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: RequestUserInputInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("request_user_input 参数非法：{}", error))?;
        if input.questions.is_empty() || input.questions.len() > 3 {
            return Err(anyhow!("request_user_input 需要 1 到 3 个问题"));
        }

        for question in &input.questions {
            if question.header.trim().is_empty()
                || question.id.trim().is_empty()
                || question.question.trim().is_empty()
            {
                return Err(anyhow!("request_user_input 存在空白问题字段"));
            }
            if question.options.len() < 2 || question.options.len() > 3 {
                return Err(anyhow!("request_user_input 的每个问题需要 2 到 3 个选项"));
            }
            if question.options.iter().any(|option| {
                option.label.trim().is_empty() || option.description.trim().is_empty()
            }) {
                return Err(anyhow!("request_user_input 存在空白选项字段"));
            }
        }

        Ok(serde_json::to_string_pretty(&json!({
            "kind": "request_user_input",
            "questions": input.questions
        }))?)
    }
}

#[async_trait]
impl ToolHandler for RunShellTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RunShellInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("run_shell 参数非法：{}", error))?;
        let timeout_ms = input.timeout_ms.unwrap_or(10_000);
        let working_directory = input
            .working_directory
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());

        let mut command = Command::new(&context.shell_program);
        command.kill_on_drop(true);
        command
            .arg("-NoProfile")
            .arg("-Command")
            .arg(&input.command)
            .current_dir(&working_directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = timeout(Duration::from_millis(timeout_ms), command.output())
            .await
            .map_err(|_| anyhow!("Shell 命令执行超时，{} 毫秒内未完成", timeout_ms))?
            .with_context(|| format!("执行 Shell 命令失败：{}", input.command))?;

        Ok(render_process_output(
            &output.stdout,
            &output.stderr,
            output.status.code(),
        ))
    }
}

#[async_trait]
impl ToolHandler for RlmOpenTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RlmOpenInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("rlm_open 参数非法：{}", error))?;
        let working_directory = input
            .working_directory
            .map(|value| resolve_path(&context.workspace_root, &value))
            .unwrap_or_else(|| context.workspace_root.clone());

        let mut command = Command::new(&context.shell_program);
        command
            .arg("-NoProfile")
            .arg("-NoLogo")
            .arg("-Command")
            .arg("-")
            .current_dir(&working_directory)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("启动 RLM 进程失败")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("无法获取 RLM stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("无法获取 RLM stdout"))?;
        let stdout = BufReader::new(stdout);

        let process_id = Uuid::new_v4().to_string();
        let process = ManagedRlmProcess {
            working_directory: working_directory.clone(),
            child,
            stdin,
            stdout,
        };
        rlm_registry()
            .lock()
            .await
            .insert(process_id.clone(), process);

        let record = RlmProcessRecord {
            id: process_id.clone(),
            shell_program: context.shell_program.clone(),
            working_directory: working_directory.to_string_lossy().to_string(),
            status: "open".to_string(),
            created_at: crate::utils::time::now_rfc3339(),
            updated_at: crate::utils::time::now_rfc3339(),
        };
        persist_rlm_record(&context.session_dir, record)?;
        Ok(serde_json::json!({
            "process_id": process_id,
            "working_directory": working_directory,
            "status": "open"
        })
        .to_string())
    }
}

#[async_trait]
impl ToolHandler for RlmEvalTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RlmEvalInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("rlm_eval 参数非法：{}", error))?;
        let sentinel = format!("__DSHNS_RLM_DONE_{}__", Uuid::new_v4());
        let mut registry = rlm_registry().lock().await;
        let process = registry
            .get_mut(&input.process_id)
            .ok_or_else(|| anyhow!("未找到 RLM 进程：{}", input.process_id))?;

        process
            .stdin
            .write_all(format!("{}\nWrite-Output \"{}\"\n", input.command, sentinel).as_bytes())
            .await
            .context("写入 RLM 命令失败")?;
        process.stdin.flush().await.context("刷新 RLM stdin 失败")?;

        let mut output = String::new();
        loop {
            let mut line = String::new();
            let size = process
                .stdout
                .read_line(&mut line)
                .await
                .context("读取 RLM 输出失败")?;
            if size == 0 {
                break;
            }
            if line.trim() == sentinel {
                break;
            }
            output.push_str(&line);
        }

        touch_rlm_record(&context.session_dir, &input.process_id, None, Some("open"))?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl ToolHandler for RlmConfigureTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RlmConfigureInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("rlm_configure 参数非法：{}", error))?;
        let new_directory = input
            .working_directory
            .map(|value| resolve_path(&context.workspace_root, &value));

        let mut registry = rlm_registry().lock().await;
        let process = registry
            .get_mut(&input.process_id)
            .ok_or_else(|| anyhow!("未找到 RLM 进程：{}", input.process_id))?;

        if let Some(directory) = new_directory {
            let command = format!("Set-Location -LiteralPath '{}'", directory.display());
            process
                .stdin
                .write_all(format!("{}\n", command).as_bytes())
                .await
                .context("更新 RLM 工作目录失败")?;
            process.stdin.flush().await.context("刷新 RLM stdin 失败")?;
            process.working_directory = directory.clone();
            touch_rlm_record(
                &context.session_dir,
                &input.process_id,
                Some(directory.to_string_lossy().to_string()),
                Some("open"),
            )?;
        }

        Ok(serde_json::json!({
            "process_id": input.process_id,
            "status": "open"
        })
        .to_string())
    }
}

#[async_trait]
impl ToolHandler for RlmCloseTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RlmCloseInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("rlm_close 参数非法：{}", error))?;
        let mut registry = rlm_registry().lock().await;
        let mut process = registry
            .remove(&input.process_id)
            .ok_or_else(|| anyhow!("未找到 RLM 进程：{}", input.process_id))?;
        process.child.kill().await.context("关闭 RLM 进程失败")?;
        touch_rlm_record(
            &context.session_dir,
            &input.process_id,
            None,
            Some("closed"),
        )?;
        Ok(serde_json::json!({
            "process_id": input.process_id,
            "status": "closed"
        })
        .to_string())
    }
}

#[async_trait]
impl ToolHandler for LoadSkillTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: LoadSkillInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("load_skill 参数非法：{}", error))?;
        context.skill_manager.load_skill(&input.identifier)
    }
}

pub struct RetrieveToolResultTool;
pub struct HandleReadTool;
pub struct FinanceTool;

#[async_trait]
impl ToolHandler for ReadToolResultTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ReadToolResultInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("read_tool_result 参数非法：{}", error))?;
        let index_path = context.session_dir.join("tool_results").join("index.json");
        let content = read_optional_utf8(&index_path)?
            .ok_or_else(|| anyhow!("工具结果索引不存在：{}", index_path.display()))?;
        let records: Vec<crate::domain::ToolResultRecord> =
            serde_json::from_str(&content).unwrap_or_default();
        let record = records
            .into_iter()
            .find(|record| record.handle == input.handle)
            .ok_or_else(|| anyhow!("未找到工具结果句柄：{}", input.handle))?;

        if record.externalized {
            let body_path = PathBuf::from(&record.body_file_path);
            let body = read_optional_utf8(&body_path)?
                .ok_or_else(|| anyhow!("工具结果正文不存在：{}", body_path.display()))?;
            Ok(body)
        } else {
            Ok(record.projection_content)
        }
    }
}

#[async_trait]
impl ToolHandler for RetrieveToolResultTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: RetrieveToolResultInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("retrieve_tool_result 参数非法：{}", error))?;
        let record = find_tool_result_record(&context.session_dir, &input.tool_call_id)?;
        let mode = input.mode.as_deref().unwrap_or("body");

        match mode {
            "summary" => Ok(record.summary),
            "head" => Ok(record.preview_head),
            "tail" => Ok(record.preview_tail),
            "body" => read_tool_result_record_body(&record),
            "slice" => {
                let start_char = input.start_char.unwrap_or(0);
                let length_chars = input.length_chars.ok_or_else(|| {
                    anyhow!("retrieve_tool_result 在 slice 模式下缺少 length_chars")
                })?;
                let body = read_tool_result_record_body(&record)?;
                Ok(slice_by_chars(&body, start_char, length_chars))
            }
            "keyword_context" => {
                let keyword = input
                    .keyword
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        anyhow!("retrieve_tool_result 在 keyword_context 模式下缺少 keyword")
                    })?;
                let context_chars = input.context_chars.unwrap_or(80);
                let body = read_tool_result_record_body(&record)?;
                extract_keyword_context(&body, &keyword, context_chars)
                    .ok_or_else(|| anyhow!("未在工具结果中找到关键字：{}", keyword))
            }
            _ => Err(anyhow!("retrieve_tool_result 不支持的 mode：{}", mode)),
        }
    }
}

#[async_trait]
impl ToolHandler for HandleReadTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: HandleReadInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("handle_read 参数非法：{}", error))?;
        let output = if let Some(handle) = input.handle.strip_prefix("tool:") {
            let record = find_tool_result_record(&context.session_dir, handle)?;
            read_tool_result_record_body(&record)?
        } else if let Some(raw_path) = input.handle.strip_prefix("file:") {
            let path = resolve_path(&context.workspace_root, raw_path);
            let workspace_root = context
                .workspace_root
                .canonicalize()
                .unwrap_or_else(|_| context.workspace_root.clone());
            let canonical_path = path
                .canonicalize()
                .map_err(|_| anyhow!("文件句柄指向的目标不存在：{}", path.display()))?;
            if !canonical_path.starts_with(&workspace_root) {
                return Err(anyhow!(
                    "file: 句柄超出工作区范围，禁止读取：{}",
                    canonical_path.display()
                ));
            }
            read_optional_utf8(&canonical_path)?
                .ok_or_else(|| anyhow!("文件句柄指向的目标不存在：{}", canonical_path.display()))?
        } else {
            return Err(anyhow!(
                "handle_read 仅支持 tool: 或 file: 句柄：{}",
                input.handle
            ));
        };

        Ok(truncate_chars(&output, input.max_chars))
    }
}

#[async_trait]
impl ToolHandler for FinanceTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: FinanceInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("finance 参数非法：{}", error))?;
        if input.symbol.trim().is_empty() {
            return Err(anyhow!("finance 的 symbol 不能为空"));
        }

        let base_url = std::env::var("YAHOO_FINANCE_BASE_URL")
            .unwrap_or_else(|_| "https://query1.finance.yahoo.com/v8/finance/chart".to_string());
        let url = format!("{}/{}", base_url.trim_end_matches('/'), input.symbol.trim());
        let client = Client::new();
        let payload: Value = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("调用 Yahoo Finance 失败：{}", url))?
            .error_for_status()
            .with_context(|| format!("Yahoo Finance 返回失败状态：{}", url))?
            .json()
            .await
            .with_context(|| format!("解析 Yahoo Finance 响应失败：{}", url))?;

        let result = payload
            .get("chart")
            .and_then(|value| value.get("result"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| anyhow!("Yahoo Finance 响应缺少 result 字段"))?;
        let meta = result
            .get("meta")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("Yahoo Finance 响应缺少 meta 字段"))?;

        let item = FinanceQuoteItem {
            symbol: meta
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or(input.symbol.trim())
                .to_string(),
            currency: meta
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            regular_market_price: meta
                .get("regularMarketPrice")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("Yahoo Finance 响应缺少 regularMarketPrice"))?,
            previous_close: meta.get("previousClose").and_then(Value::as_f64),
            timestamp: result
                .get("timestamp")
                .and_then(Value::as_array)
                .and_then(|items| items.last())
                .and_then(Value::as_i64),
        };
        Ok(serde_json::to_string_pretty(&item)?)
    }
}

#[async_trait]
impl ToolHandler for FetchUrlTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: FetchUrlInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("fetch_url 参数非法：{}", error))?;
        let client = Client::new();
        client
            .get(&input.url)
            .send()
            .await
            .with_context(|| format!("抓取 URL 失败：{}", input.url))?
            .error_for_status()
            .with_context(|| format!("URL 返回失败状态：{}", input.url))?
            .text()
            .await
            .with_context(|| format!("读取 URL 正文失败：{}", input.url))
    }
}

#[async_trait]
impl ToolHandler for GithubGetTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: GithubGetInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("github_get 参数非法：{}", error))?;
        let token = read_required_env("GITHUB_TOKEN")?;
        let endpoint = if input.endpoint.starts_with('/') {
            input.endpoint
        } else {
            format!("/{}", input.endpoint)
        };
        let base_url = std::env::var("GITHUB_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.github.com".to_string());
        let url = format!("{}{}", base_url.trim_end_matches('/'), endpoint);
        let client = Client::new();
        client
            .get(&url)
            .header("User-Agent", "dshns-rust-harness")
            .bearer_auth(token)
            .send()
            .await
            .with_context(|| format!("调用 GitHub 接口失败：{}", url))?
            .error_for_status()
            .with_context(|| format!("GitHub 接口返回失败状态：{}", url))?
            .text()
            .await
            .with_context(|| format!("读取 GitHub 接口响应失败：{}", url))
    }
}

#[async_trait]
impl ToolHandler for WebSearchTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: WebSearchInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("web_search 参数非法：{}", error))?;
        let client = Client::new();
        let rss = client
            .get("https://www.bing.com/search")
            .query(&[("format", "rss"), ("q", input.query.as_str())])
            .send()
            .await
            .context("执行网络搜索失败")?
            .error_for_status()
            .context("搜索接口返回失败状态")?
            .text()
            .await
            .context("读取搜索结果正文失败")?;

        let mut items = parse_bing_rss(&rss);
        if let Some(limit) = input.limit {
            items.truncate(limit);
        }
        Ok(serde_json::to_string_pretty(&items)?)
    }
}

#[async_trait]
impl ToolHandler for WebRunTool {
    async fn handle(&self, input: Value, _context: &ToolExecutionContext) -> Result<String> {
        let input: WebRunInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("web_run 参数非法：{}", error))?;
        if input.steps.is_empty() {
            return Err(anyhow!("web_run 至少需要一个步骤"));
        }

        let client = Client::new();
        let mut current_url = String::new();
        let mut current_html = String::new();
        let mut results = Vec::new();

        for step in input.steps {
            match step {
                WebRunStep::Open { url } => {
                    let html = fetch_text(&client, &url).await?;
                    current_url = url.clone();
                    current_html = html;
                    results.push(WebRunStepResult {
                        step: "open".to_string(),
                        url,
                        success: true,
                        output: "已打开页面".to_string(),
                    });
                }
                WebRunStep::Click { text_contains } => {
                    ensure_page_loaded(&current_url)?;
                    let next_url = find_link_by_text(&current_url, &current_html, &text_contains)?
                        .ok_or_else(|| anyhow!("未找到包含指定文本的链接：{}", text_contains))?;
                    let html = fetch_text(&client, &next_url).await?;
                    current_url = next_url.clone();
                    current_html = html;
                    results.push(WebRunStepResult {
                        step: "click".to_string(),
                        url: current_url.clone(),
                        success: true,
                        output: format!("已点击并打开链接，匹配文本：{}", text_contains),
                    });
                }
                WebRunStep::Find { pattern } => {
                    ensure_page_loaded(&current_url)?;
                    let hits = extract_visible_text(&current_html)
                        .matches(&pattern)
                        .count();
                    results.push(WebRunStepResult {
                        step: "find".to_string(),
                        url: current_url.clone(),
                        success: hits > 0,
                        output: format!("匹配次数：{}", hits),
                    });
                }
                WebRunStep::ExtractText { selector } => {
                    ensure_page_loaded(&current_url)?;
                    let text = if let Some(selector) = selector {
                        extract_text_by_selector(&current_html, &selector)?
                    } else {
                        extract_visible_text(&current_html)
                    };
                    results.push(WebRunStepResult {
                        step: "extract_text".to_string(),
                        url: current_url.clone(),
                        success: true,
                        output: text,
                    });
                }
            }
        }

        Ok(serde_json::to_string_pretty(&results)?)
    }
}

#[async_trait]
impl ToolHandler for DiscoverMcpServersTool {
    async fn handle(&self, _input: Value, context: &ToolExecutionContext) -> Result<String> {
        let manager =
            McpClientManager::new(context.workspace_root.clone(), context.session_dir.clone());
        let servers = manager.discover_servers()?;
        Ok(serde_json::to_string_pretty(&servers)?)
    }
}

#[async_trait]
impl ToolHandler for ConnectMcpServerTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: ConnectMcpInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("connect_mcp_server 参数非法：{}", error))?;
        let manager =
            McpClientManager::new(context.workspace_root.clone(), context.session_dir.clone());
        let state = manager.connect_server(&input.server_id).await?;
        Ok(serde_json::to_string_pretty(&state)?)
    }
}

#[async_trait]
impl ToolHandler for CallMcpTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: CallMcpToolInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("call_mcp_tool 参数非法：{}", error))?;
        let manager =
            McpClientManager::new(context.workspace_root.clone(), context.session_dir.clone());
        let result = manager
            .call_tool(&input.server_id, &input.tool_name, input.arguments)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }
}

#[async_trait]
impl ToolHandler for AgentOpenTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: AgentOpenInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("agent_open 参数非法：{}", error))?;
        let manager = SubagentManager::new(context.session_dir.clone());
        let agent = manager.open(
            &input.mode,
            input.inherit_context.unwrap_or(input.mode != "isolate"),
            input.allowed_paths.unwrap_or_default(),
            input.task_spec,
            input.parent_agent_id,
        )?;
        let (session_id, round_no) = current_session_event_context(&context.session_dir);
        EventBus::new(context.session_dir.clone()).emit_agent_status(
            &session_id,
            round_no,
            &agent.id,
            "open",
            Some(&agent.child_session_id),
            agent.parent_agent_id.as_deref(),
        )?;
        Ok(serde_json::to_string_pretty(&agent)?)
    }
}

#[async_trait]
impl ToolHandler for AgentEvalTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: AgentEvalInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("agent_eval 参数非法：{}", error))?;
        let manager = SubagentManager::new(context.session_dir.clone());
        let (session_id, round_no) = current_session_event_context(&context.session_dir);
        EventBus::new(context.session_dir.clone()).emit_agent_status(
            &session_id,
            round_no,
            &input.agent_id,
            "running",
            None,
            None,
        )?;
        let result = manager.eval(&input.agent_id, input.input)?;
        let child_session_id = result
            .get("child_session_id")
            .and_then(serde_json::Value::as_str);
        EventBus::new(context.session_dir.clone()).emit_agent_status(
            &session_id,
            round_no,
            &input.agent_id,
            "done",
            child_session_id,
            None,
        )?;
        Ok(serde_json::to_string_pretty(&result)?)
    }
}

#[async_trait]
impl ToolHandler for AgentCloseTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: AgentCloseInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("agent_close 参数非法：{}", error))?;
        let manager = SubagentManager::new(context.session_dir.clone());
        let result = manager.close(&input.agent_id)?;
        let (session_id, round_no) = current_session_event_context(&context.session_dir);
        EventBus::new(context.session_dir.clone()).emit_agent_status(
            &session_id,
            round_no,
            &result.id,
            "closed",
            Some(&result.child_session_id),
            result.parent_agent_id.as_deref(),
        )?;
        Ok(serde_json::to_string_pretty(&result)?)
    }
}

#[async_trait]
impl ToolHandler for PlanWriteTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: PlanWriteInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("plan_write 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("plan")
            .join(format!("{}.md", sanitize_name(&input.plan_type)));
        write_utf8(&path, &input.content)?;
        Ok(format!("计划文件已写入：{}", path.display()))
    }
}

#[async_trait]
impl ToolHandler for TaskCreateTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: TaskCreateInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("task_create 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("task")
            .join("tasks.json");
        let mut records = load_json_array::<TaskRecord>(&path)?;
        let record = TaskRecord {
            id: Uuid::new_v4().to_string(),
            name: input.name,
            status: "created".to_string(),
            command: input.command,
            validation_command: input.validation_command,
            bound_process_id: None,
            created_at: crate::utils::time::now_rfc3339(),
        };
        records.push(record.clone());
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
        Ok(serde_json::to_string_pretty(&record)?)
    }
}

pub struct TaskShellStartTool;
pub struct TaskShellWaitTool;

#[async_trait]
impl ToolHandler for TaskRunTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: TaskRunInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("task_run 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("task")
            .join("tasks.json");
        let mut records = load_json_array::<TaskRecord>(&path)?;
        let index = records
            .iter()
            .position(|record| record.id == input.task_id)
            .ok_or_else(|| anyhow!("未找到任务：{}", input.task_id))?;
        records[index].status = "running".to_string();
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;

        let output = run_shell_command(
            &context.shell_program,
            &records[index].command,
            &context.workspace_root,
            input.timeout_ms.unwrap_or(10_000),
        )
        .await?;

        if let Some(validation_command) = records[index].validation_command.clone() {
            let validation_output = run_shell_command(
                &context.shell_program,
                &validation_command,
                &context.workspace_root,
                input.timeout_ms.unwrap_or(10_000),
            )
            .await?;
            records[index].status = "validated".to_string();
            write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
            return Ok(format!(
                "任务执行完成。\n主命令结果：\n{}\n\n验证命令结果：\n{}",
                output, validation_output
            ));
        }

        records[index].status = "done".to_string();
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
        Ok(output)
    }
}

#[async_trait]
impl ToolHandler for TaskShellStartTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: TaskShellStartInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("task_shell_start 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("task")
            .join("tasks.json");
        let mut records = load_json_array::<TaskRecord>(&path)?;
        let index = records
            .iter()
            .position(|record| record.id == input.task_id)
            .ok_or_else(|| anyhow!("未找到任务：{}", input.task_id))?;

        let exec_tool = ExecShellTool;
        let payload = if let Some(working_directory) = input.working_directory {
            json!({
                "command": records[index].command,
                "working_directory": working_directory
            })
        } else {
            json!({
                "command": records[index].command
            })
        };
        let result = exec_tool.handle(payload, context).await?;
        let payload: Value = serde_json::from_str(&result)
            .map_err(|error| anyhow!("解析 task_shell_start 结果失败：{}", error))?;
        let process_id = payload
            .get("process_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("task_shell_start 缺少 process_id"))?
            .to_string();

        records[index].status = "running".to_string();
        records[index].bound_process_id = Some(process_id);
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
        Ok(result)
    }
}

#[async_trait]
impl ToolHandler for TaskShellWaitTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: TaskShellWaitInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("task_shell_wait 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("task")
            .join("tasks.json");
        let mut records = load_json_array::<TaskRecord>(&path)?;
        let index = records
            .iter()
            .position(|record| record.id == input.task_id)
            .ok_or_else(|| anyhow!("未找到任务：{}", input.task_id))?;
        let process_id = records[index]
            .bound_process_id
            .clone()
            .ok_or_else(|| anyhow!("当前任务未绑定后台进程：{}", input.task_id))?;

        let wait_tool = ExecShellWaitTool;
        let output = wait_tool
            .handle(
                json!({
                    "process_id": process_id,
                    "idle_timeout_ms": input.idle_timeout_ms,
                    "max_lines": input.max_lines
                }),
                context,
            )
            .await?;
        let cancel_tool = ExecShellCancelTool;
        let _ = cancel_tool
            .handle(
                json!({
                    "process_id": process_id
                }),
                context,
            )
            .await;
        records[index].status = "done".to_string();
        records[index].bound_process_id = None;
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
        Ok(output)
    }
}

#[async_trait]
impl ToolHandler for AutomationCreateTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: AutomationCreateInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("automation_create 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("automation")
            .join("automations.json");
        let mut records = load_json_array::<AutomationRecord>(&path)?;
        let record = AutomationRecord {
            id: Uuid::new_v4().to_string(),
            name: input.name,
            kind: input.kind,
            status: input.status,
            definition_json: serde_json::to_string(&input.definition)?,
            last_run_at: None,
        };
        records.push(record.clone());
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
        Ok(serde_json::to_string_pretty(&record)?)
    }
}

#[async_trait]
impl ToolHandler for AutomationRunOnceTool {
    async fn handle(&self, input: Value, context: &ToolExecutionContext) -> Result<String> {
        let input: AutomationRunOnceInput = serde_json::from_value(input)
            .map_err(|error| anyhow!("automation_run_once 参数非法：{}", error))?;
        let path = context
            .session_dir
            .join(".tools")
            .join("automation")
            .join("automations.json");
        let mut records = load_json_array::<AutomationRecord>(&path)?;
        let index = records
            .iter()
            .position(|record| record.id == input.automation_id)
            .ok_or_else(|| anyhow!("未找到自动化：{}", input.automation_id))?;
        let definition: Value = serde_json::from_str(&records[index].definition_json)?;
        let command = definition
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("自动化定义缺少 command 字段"))?;
        let working_directory = definition
            .get("working_directory")
            .and_then(Value::as_str)
            .map(|value| resolve_path(&context.workspace_root, value))
            .unwrap_or_else(|| context.workspace_root.clone());

        let output = run_shell_command(
            &context.shell_program,
            command,
            &working_directory,
            input.timeout_ms.unwrap_or(10_000),
        )
        .await?;

        records[index].last_run_at = Some(crate::utils::time::now_rfc3339());
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
        Ok(output)
    }
}

fn resolve_path(workspace_root: &Path, raw_path: &str) -> PathBuf {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn find_tool_result_record(
    session_dir: &Path,
    tool_call_id: &str,
) -> Result<crate::domain::ToolResultRecord> {
    let index_path = session_dir.join("tool_results").join("index.json");
    let content = read_optional_utf8(&index_path)?
        .ok_or_else(|| anyhow!("工具结果索引不存在：{}", index_path.display()))?;
    let records: Vec<crate::domain::ToolResultRecord> =
        serde_json::from_str(&content).unwrap_or_default();
    records
        .into_iter()
        .find(|record| {
            record.tool_call_id == tool_call_id || record.handle == format!("tool:{}", tool_call_id)
        })
        .ok_or_else(|| anyhow!("未找到工具结果：{}", tool_call_id))
}

fn read_tool_result_record_body(record: &crate::domain::ToolResultRecord) -> Result<String> {
    if record.externalized {
        let body_path = PathBuf::from(&record.body_file_path);
        read_optional_utf8(&body_path)?
            .ok_or_else(|| anyhow!("工具结果正文不存在：{}", body_path.display()))
    } else {
        Ok(record.projection_content.clone())
    }
}

fn slice_by_chars(content: &str, start_char: usize, length_chars: usize) -> String {
    content
        .chars()
        .skip(start_char)
        .take(length_chars)
        .collect()
}

fn truncate_chars(content: &str, max_chars: Option<usize>) -> String {
    match max_chars {
        Some(limit) => content.chars().take(limit).collect(),
        None => content.to_string(),
    }
}

fn extract_keyword_context(content: &str, keyword: &str, context_chars: usize) -> Option<String> {
    let chars = content.chars().collect::<Vec<_>>();
    let keyword_chars = keyword.chars().collect::<Vec<_>>();
    if keyword_chars.is_empty() {
        return None;
    }

    let match_index = chars
        .windows(keyword_chars.len())
        .position(|window| window == keyword_chars.as_slice())?;
    let start = match_index.saturating_sub(context_chars);
    let end = (match_index + keyword_chars.len() + context_chars).min(chars.len());
    Some(chars[start..end].iter().collect())
}

fn current_session_event_context(session_dir: &Path) -> (String, i64) {
    match read_session_ini(session_dir) {
        Ok(session) => (session.id, session.round + 1),
        Err(_) => (
            session_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown-session")
                .to_string(),
            1,
        ),
    }
}

async fn run_shell_command(
    shell_program: &str,
    command_text: &str,
    working_directory: &Path,
    timeout_ms: u64,
) -> Result<String> {
    let mut command = Command::new(shell_program);
    command.kill_on_drop(true);
    command
        .arg("-NoProfile")
        .arg("-Command")
        .arg(command_text)
        .current_dir(working_directory)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = timeout(Duration::from_millis(timeout_ms), command.output())
        .await
        .map_err(|_| anyhow!("Shell 命令执行超时，{} 毫秒内未完成", timeout_ms))?
        .with_context(|| format!("执行 Shell 命令失败：{}", command_text))?;

    Ok(render_process_output(
        &output.stdout,
        &output.stderr,
        output.status.code(),
    ))
}

fn render_process_output(stdout: &[u8], stderr: &[u8], code: Option<i32>) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    let mut rendered = format!("退出码：{}\n", code.unwrap_or(-1));
    if !stdout.is_empty() {
        rendered.push_str(&format!("标准输出：\n{}\n", stdout));
    }
    if !stderr.is_empty() {
        rendered.push_str(&format!("标准错误：\n{}\n", stderr));
    }
    rendered.trim().to_string()
}

fn load_json_array<T>(path: &Path) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    let content = read_optional_utf8(path)?.unwrap_or_else(|| "[]".to_string());
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || value == '-' || value == '_' {
                value
            } else {
                '_'
            }
        })
        .collect()
}

fn rlm_registry() -> &'static Arc<Mutex<std::collections::HashMap<String, ManagedRlmProcess>>> {
    static REGISTRY: OnceLock<Arc<Mutex<std::collections::HashMap<String, ManagedRlmProcess>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
}

fn exec_registry() -> &'static Arc<Mutex<std::collections::HashMap<String, ManagedExecProcess>>> {
    static REGISTRY: OnceLock<Arc<Mutex<std::collections::HashMap<String, ManagedExecProcess>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
}

fn persist_rlm_record(session_dir: &Path, record: RlmProcessRecord) -> Result<()> {
    let path = session_dir
        .join(".tools")
        .join("rlm")
        .join("processes.json");
    let mut records = load_json_array::<RlmProcessRecord>(&path)?;
    records.push(record);
    write_utf8(&path, &serde_json::to_string_pretty(&records)?)
}

fn touch_rlm_record(
    session_dir: &Path,
    process_id: &str,
    working_directory: Option<String>,
    status: Option<&str>,
) -> Result<()> {
    let path = session_dir
        .join(".tools")
        .join("rlm")
        .join("processes.json");
    let mut records = load_json_array::<RlmProcessRecord>(&path)?;
    if let Some(index) = records.iter().position(|record| record.id == process_id) {
        if let Some(working_directory) = working_directory {
            records[index].working_directory = working_directory;
        }
        if let Some(status) = status {
            records[index].status = status.to_string();
        }
        records[index].updated_at = crate::utils::time::now_rfc3339();
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
    }
    Ok(())
}

fn persist_exec_record(session_dir: &Path, record: ExecProcessRecord) -> Result<()> {
    let path = session_dir
        .join(".tools")
        .join("exec")
        .join("processes.json");
    let mut records = load_json_array::<ExecProcessRecord>(&path)?;
    records.push(record);
    write_utf8(&path, &serde_json::to_string_pretty(&records)?)
}

fn touch_exec_record(
    session_dir: &Path,
    process_id: &str,
    working_directory: Option<String>,
    status: Option<&str>,
) -> Result<()> {
    let path = session_dir
        .join(".tools")
        .join("exec")
        .join("processes.json");
    let mut records = load_json_array::<ExecProcessRecord>(&path)?;
    if let Some(index) = records.iter().position(|record| record.id == process_id) {
        if let Some(working_directory) = working_directory {
            records[index].working_directory = working_directory;
        }
        if let Some(status) = status {
            records[index].status = status.to_string();
        }
        records[index].updated_at = crate::utils::time::now_rfc3339();
        write_utf8(&path, &serde_json::to_string_pretty(&records)?)?;
    }
    Ok(())
}

fn parse_bing_rss(content: &str) -> Vec<SearchResultItem> {
    let mut items = Vec::new();
    let mut rest = content;

    while let Some(start) = rest.find("<item>") {
        let after_start = &rest[start + "<item>".len()..];
        let Some(end) = after_start.find("</item>") else {
            break;
        };
        let block = &after_start[..end];
        let title = extract_xml_tag(block, "title").unwrap_or_default();
        let link = extract_xml_tag(block, "link").unwrap_or_default();
        let snippet = extract_xml_tag(block, "description").unwrap_or_default();
        items.push(SearchResultItem {
            title: decode_xml_entities(&title),
            link: decode_xml_entities(&link),
            snippet: decode_xml_entities(&snippet),
        });
        rest = &after_start[end + "</item>".len()..];
    }

    items
}

async fn fetch_text(client: &Client, url: &str) -> Result<String> {
    client
        .get(url)
        .send()
        .await
        .with_context(|| format!("访问网页失败：{}", url))?
        .error_for_status()
        .with_context(|| format!("网页返回失败状态：{}", url))?
        .text()
        .await
        .with_context(|| format!("读取网页内容失败：{}", url))
}

fn ensure_page_loaded(current_url: &str) -> Result<()> {
    if current_url.is_empty() {
        Err(anyhow!("当前还没有打开任何网页，请先执行 open 步骤"))
    } else {
        Ok(())
    }
}

fn find_link_by_text(base_url: &str, html: &str, text_contains: &str) -> Result<Option<String>> {
    let document = Html::parse_document(html);
    let selector =
        Selector::parse("a").map_err(|error| anyhow!("解析链接选择器失败：{}", error))?;
    let base = Url::parse(base_url).with_context(|| format!("解析当前 URL 失败：{}", base_url))?;

    for link in document.select(&selector) {
        let text = link.text().collect::<Vec<_>>().join(" ");
        if !text.contains(text_contains) {
            continue;
        }
        if let Some(href) = link.value().attr("href") {
            let resolved = base
                .join(href)
                .with_context(|| format!("拼接链接失败：{}", href))?;
            return Ok(Some(resolved.to_string()));
        }
    }

    Ok(None)
}

fn extract_visible_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let selector = Selector::parse("body").expect("body 选择器必须合法");
    document
        .select(&selector)
        .flat_map(|node| node.text())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_text_by_selector(html: &str, selector_text: &str) -> Result<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(selector_text)
        .map_err(|error| anyhow!("解析 CSS 选择器失败：{}，选择器：{}", error, selector_text))?;
    let text = document
        .select(&selector)
        .flat_map(|node| node.text())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn extract_xml_tag(block: &str, tag: &str) -> Option<String> {
    let start_token = format!("<{}>", tag);
    let end_token = format!("</{}>", tag);
    let start = block.find(&start_token)?;
    let tail = &block[start + start_token.len()..];
    let end = tail.find(&end_token)?;
    Some(tail[..end].to_string())
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("<![CDATA[", "")
        .replace("]]>", "")
}

fn read_required_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow!("缺少环境变量 {}，无法调用相关工具", name))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use crate::skill::manager::SkillManager;
    use crate::tools::registry::{ToolExecutionContext, ToolHandler};
    use crate::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};

    use super::{
        extract_text_by_selector, find_link_by_text, parse_bing_rss, read_required_env,
        PlanWriteTool, ReadToolResultTool, RlmCloseTool, RlmEvalTool, RlmOpenTool, TaskCreateTool,
    };

    fn build_context(session_dir: &str) -> ToolExecutionContext {
        let workspace_root = PathBuf::from("target/test_workspace");
        let session_dir = PathBuf::from(session_dir);
        ensure_directory(&workspace_root).expect("创建测试工作区失败");
        ensure_directory(&session_dir).expect("创建测试会话目录失败");

        ToolExecutionContext {
            workspace_root,
            session_dir,
            shell_program: "powershell".to_string(),
            skill_manager: SkillManager::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn should_write_plan_file() {
        let context = build_context("target/test_plan_session");
        let tool = PlanWriteTool;
        let result = tool
            .handle(
                json!({
                    "plan_type": "implementation",
                    "content": "# 计划\n- 第一步"
                }),
                &context,
            )
            .await
            .expect("计划写入失败");
        assert!(result.contains("implementation.md"));
    }

    #[tokio::test]
    async fn should_create_task_record() {
        let context = build_context("target/test_task_session");
        let tool = TaskCreateTool;
        tool.handle(
            json!({
                "name": "demo",
                "command": "Write-Output 'hello'"
            }),
            &context,
        )
        .await
        .expect("任务创建失败");

        let content = read_optional_utf8(
            &context
                .session_dir
                .join(".tools")
                .join("task")
                .join("tasks.json"),
        )
        .expect("读取任务文件失败")
        .expect("任务文件不存在");
        assert!(content.contains("\"name\": \"demo\""));
    }

    #[tokio::test]
    async fn should_read_tool_result_handle() {
        let context = build_context("target/test_tool_result_session");
        let index_path = context.session_dir.join("tool_results").join("index.json");
        write_utf8(
            &index_path,
            r#"[{"tool_call_id":"call_1","tool_name":"read_file","handle":"tool:call_1","body_file_path":"","projection_type":"InlineFull","projection_content":"hello","summary":"ok","preview_head":"","preview_tail":"","char_count":5,"byte_count":5,"success":true,"truncated":false,"externalized":false,"updated_at":"2026-01-01T00:00:00Z"}]"#,
        )
        .expect("写入工具结果索引失败");

        let tool = ReadToolResultTool;
        let result = tool
            .handle(json!({"handle": "tool:call_1"}), &context)
            .await
            .expect("读取句柄失败");
        assert_eq!(result, "hello");
    }

    #[test]
    fn should_parse_bing_rss_results() {
        let rss = r#"
        <rss>
          <channel>
            <item>
              <title>Rust language</title>
              <link>https://www.rust-lang.org/</link>
              <description>Systems programming language</description>
            </item>
          </channel>
        </rss>
        "#;
        let items = parse_bing_rss(rss);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Rust language");
        assert_eq!(items[0].link, "https://www.rust-lang.org/");
    }

    #[test]
    fn should_find_link_and_extract_text_from_html() {
        let html = r#"
        <html>
          <body>
            <a href="/next">Go next</a>
            <div class="content">Hello Rust</div>
          </body>
        </html>
        "#;
        let url = find_link_by_text("https://example.com/start", html, "Go next")
            .expect("查找链接失败")
            .expect("未找到链接");
        assert_eq!(url, "https://example.com/next");

        let text = extract_text_by_selector(html, ".content").expect("提取文本失败");
        assert_eq!(text, "Hello Rust");
    }

    #[tokio::test]
    async fn should_open_eval_and_close_rlm_process() {
        let context = build_context(&format!("target/test_rlm_session_{}", uuid::Uuid::new_v4()));

        let open_tool = RlmOpenTool;
        let open_result = open_tool
            .handle(json!({}), &context)
            .await
            .expect("打开 RLM 进程失败");
        let open_json: serde_json::Value =
            serde_json::from_str(&open_result).expect("解析 rlm_open 结果失败");
        let process_id = open_json
            .get("process_id")
            .and_then(serde_json::Value::as_str)
            .expect("缺少 process_id")
            .to_string();

        let eval_tool = RlmEvalTool;
        let eval_result = eval_tool
            .handle(
                json!({
                    "process_id": process_id,
                    "command": "Write-Output 'hello from rlm'"
                }),
                &context,
            )
            .await
            .expect("执行 rlm_eval 失败");
        assert!(eval_result.contains("hello from rlm"));

        let close_tool = RlmCloseTool;
        let close_result = close_tool
            .handle(json!({ "process_id": process_id }), &context)
            .await
            .expect("关闭 RLM 进程失败");
        assert!(close_result.contains("closed"));
    }

    #[test]
    fn should_require_missing_env() {
        let error =
            read_required_env("DSHNS_TEST_MISSING_ENV").expect_err("应当返回缺少环境变量错误");
        assert!(error.to_string().contains("缺少环境变量"));
    }
}
