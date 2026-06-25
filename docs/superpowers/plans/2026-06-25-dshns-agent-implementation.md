# dsHns DeepSeek Agent 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建类似 Claude Code 的命令行 AI 编程助手，底层对接 DeepSeek API。

**Architecture:** Rust workspace 6 crate，事件驱动管道。`core` 是唯一共享依赖。Agent 循环基于 SSE 流式处理 + 双层安全检查。

**Tech Stack:** Rust 2021, tokio, reqwest, serde/serde_json, clap, rustyline, wiremock

## Global Constraints

- 纯 Rust 2021 edition
- Shell：`powershell.exe -NoProfile -Command "chcp 65001 > $null; <cmd>"`
- 编码：UTF-8 无 BOM 写入，cp65001 终端
- 配置目录：`~/.dsHns_rs/`
- API Key：环境变量 `DEEPSEEK_API_KEY`
- 默认模型：`deepseek-v4-flash`
- 审批：`Remove-Item -Recurse -Force` / `del /f /s /q` 硬拒绝，不弹审批
- 子智能体 depth=0，不可创建孙智能体
- 禁止循环依赖，core 是唯一公共依赖

---

## 文件结构总览

```
dshns/
├── Cargo.toml                    # workspace root
├── crates/
│   ├── core/src/                 # 7 files
│   ├── deepseek-client/src/      # 4 files
│   ├── tools/src/                # 7 files
│   ├── session-store/src/        # 4 files
│   ├── agent/src/                # 6 files
│   └── app/src/                  # 3 files
└── tests/
```

---

### Task 1: Cargo Workspace 骨架搭建

**Files to create:** 根 `Cargo.toml`, 6 个 crate 的 `Cargo.toml` + `lib.rs`/`main.rs`, `.gitignore`

- [ ] **Step 1: 创建根 Cargo.toml**

```toml
[workspace]
members = [
    "crates/core",
    "crates/deepseek-client",
    "crates/tools",
    "crates/session-store",
    "crates/agent",
    "crates/app",
]
resolver = "2"
```

- [ ] **Step 2: 创建 core crate**

`crates/core/Cargo.toml`:
```toml
[package]
name = "dshns-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
thiserror = "1"
```

`crates/core/src/lib.rs`:
```rust
pub mod message;
pub mod session;
pub mod tool;
pub mod config;
pub mod event;
pub mod error;
```

- [ ] **Step 3: 创建 deepseek-client crate**

`crates/deepseek-client/Cargo.toml`:
```toml
[package]
name = "dshns-deepseek-client"
version = "0.1.0"
edition = "2021"

[dependencies]
dshns-core = { path = "../core" }
reqwest = { version = "0.12", features = ["json", "stream"] }
tokio = { version = "1", features = ["full"] }
serde = "1"
serde_json = "1"
tokio-stream = "0.1"
futures = "0.3"
```

- [ ] **Step 4: 创建 tools crate**

`crates/tools/Cargo.toml`:
```toml
[package]
name = "dshns-tools"
version = "0.1.0"
edition = "2021"

[dependencies]
dshns-core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde_json = "1"
regex = "1"
futures = "0.3"
```

- [ ] **Step 5: 创建 session-store crate**

```toml
[package]
name = "dshns-session-store"
version = "0.1.0"
edition = "2021"

[dependencies]
dshns-core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
serde = "1"
serde_json = "1"
chrono = "0.4"
uuid = "1"
```

- [ ] **Step 6: 创建 agent crate**

```toml
[package]
name = "dshns-agent"
version = "0.1.0"
edition = "2021"

[dependencies]
dshns-core = { path = "../core" }
dshns-deepseek-client = { path = "../deepseek-client" }
dshns-tools = { path = "../tools" }
dshns-session-store = { path = "../session-store" }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde_json = "1"
serde = "1"
regex = "1"
uuid = "1"
futures = "0.3"
```

- [ ] **Step 7: 创建 app crate**

```toml
[package]
name = "dshns"
version = "0.1.0"
edition = "2021"

[dependencies]
dshns-core = { path = "../core" }
dshns-deepseek-client = { path = "../deepseek-client" }
dshns-tools = { path = "../tools" }
dshns-session-store = { path = "../session-store" }
dshns-agent = { path = "../agent" }
clap = { version = "4", features = ["derive"] }
rustyline = "14"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
serde = "1"
chrono = "0.4"
toml = "0.8"
```

`crates/app/src/main.rs`:
```rust
fn main() {
    println!("dsHns - DeepSeek Agent");
}
```

- [ ] **Step 8: 创建 .gitignore**

```
/target/
Cargo.lock
```

- [ ] **Step 9: 验证编译**

Run: `cargo check`
Expected: 所有 crate 编译通过

- [ ] **Step 10: 提交**

```bash
git add -A && git commit -m "feat: cargo workspace skeleton (6 crates)"
```

---

### Task 2: core — Message, Session, Tool 类型

**Files:** `crates/core/src/message.rs`, `session.rs`, `tool.rs`

- [ ] **Step 1: 写 message.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCallRef>>,
    },
    #[serde(rename = "tool")]
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRef {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
```

- [ ] **Step 2: 写 session.rs**

```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    pub fn to_dir_name(&self) -> String { self.0.to_string() }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u32,
    pub working_dir: PathBuf,
}

impl SessionMeta {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            id: SessionId::new(),
            title: "新会话".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message_count: 0,
            working_dir,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub meta: SessionMeta,
    pub messages: Vec<crate::message::Message>,
}
```

- [ ] **Step 3: 写 tool.rs**

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use async_trait::async_trait;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: ToolParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParams {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: HashMap<String, ParamProp>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamProp {
    #[serde(rename = "type")]
    pub prop_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub call_id: String,
    pub status: ToolStatus,
    pub content: String,
    pub was_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolStatus {
    Success,
    Error { reason: String },
    Denied,
    Timeout,
    HardBlocked { reason: String },
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDef;
    async fn execute(&self, call: &ToolCall) -> ToolOutcome;
}
```

- [ ] **Step 4: 验证编译 + 提交**

Run: `cargo check -p dshns-core`
```bash
git add crates/core/src/{message, session, tool}.rs && git commit -m "feat(core): add Message, Session, Tool types"
```

---

### Task 3: core — Config, Error, Event 类型

**Files:** `crates/core/src/config.rs`, `error.rs`, `event.rs`

- [ ] **Step 1: 写 config.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub subagent: SubAgentConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub mode: ModeConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api: ApiConfig::default(),
            agent: AgentConfig::default(),
            subagent: SubAgentConfig::default(),
            context: ContextConfig::default(),
            mode: ModeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens_per_request: u32,
    #[serde(default = "default_timeout")]
    pub request_timeout_secs: u64,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            temperature: 0.0,
            max_tokens_per_request: default_max_tokens(),
            request_timeout_secs: default_timeout(),
        }
    }
}

fn default_model() -> String { "deepseek-v4-flash".into() }
fn default_max_tokens() -> u32 { 8192 }
fn default_timeout() -> u64 { 120 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_tool_timeout_secs")]
    pub tool_timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_tool_rounds(),
            tool_timeout_secs: default_tool_timeout_secs(),
        }
    }
}

fn default_max_tool_rounds() -> u32 { 25 }
fn default_tool_timeout_secs() -> u64 { 60 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentConfig {
    #[serde(default = "default_sub_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_sub_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_sub_msgs")]
    pub inherit_mode_max_messages: usize,
}

impl Default for SubAgentConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_sub_rounds(),
            timeout_secs: default_sub_timeout(),
            inherit_mode_max_messages: default_sub_msgs(),
        }
    }
}

fn default_sub_rounds() -> u32 { 10 }
fn default_sub_timeout() -> u64 { 300 }
fn default_sub_msgs() -> usize { 20 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_window")]
    pub max_window_tokens: usize,
    #[serde(default = "default_threshold")]
    pub compression_threshold: f64,
    #[serde(default = "default_result_tokens")]
    pub max_tool_result_tokens: usize,
    #[serde(default = "default_reserve")]
    pub reserve_tokens: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_window_tokens: default_window(),
            compression_threshold: default_threshold(),
            max_tool_result_tokens: default_result_tokens(),
            reserve_tokens: default_reserve(),
        }
    }
}

fn default_window() -> usize { 131072 }
fn default_threshold() -> f64 { 0.75 }
fn default_result_tokens() -> usize { 8000 }
fn default_reserve() -> usize { 4096 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    #[serde(default = "default_mode_val")]
    pub default: String,
}

impl Default for ModeConfig {
    fn default() -> Self { Self { default: default_mode_val() } }
}

fn default_mode_val() -> String { "auto".into() }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalMode {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "confirm")]
    Confirm,
    #[serde(rename = "paranoid")]
    Paranoid,
}

impl ApprovalMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "confirm" => Self::Confirm,
            "paranoid" => Self::Paranoid,
            _ => Self::Auto,
        }
    }
}
```

- [ ] **Step 2: 写 error.rs**

```rust
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DshnsError {
    #[error("未设置 DEEPSEEK_API_KEY 环境变量")]
    NoApiKey,
    #[error("配置错误: {0}")]
    Config(String),
    #[error("API 认证失败 (401): {0}")]
    ApiAuth(String),
    #[error("API 速率限制 (429): {0}s 后重试")]
    ApiRateLimited(u64),
    #[error("API 服务器错误: {0}")]
    ApiServer(String),
    #[error("网络错误: {0}")]
    Network(String),
    #[error("SSE 解析错误: {0}")]
    SseParse(String),
    #[error("工具执行错误: {0}")]
    Tool(String),
    #[error("会话未找到: {0}")]
    SessionNotFound(String),
    #[error("会话损坏: {0}")]
    SessionCorrupted(PathBuf),
    #[error("工具循环卡住: 连续失败 {failures} 次")]
    ToolLoopStuck { failures: u32 },
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}
```

- [ ] **Step 3: 写 event.rs**

```rust
use crate::tool::{ToolCall, ToolStatus};
use crate::message::Usage;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    UserInput(String),
    Thinking(String),
    ToolCallStart { id: String, name: String },
    ToolBlocked { call: ToolCall, reason: String },
    ToolExecution { call_id: String, status: ToolStatus, summary: String },
    ToolConfirmationNeeded { call: ToolCall, reason: String },
    SubAgentOpened { agent_id: String, mode: String, description: String },
    SubAgentCompleted { agent_id: String, summary: String },
    SubAgentClosed { agent_id: String },
    TurnComplete { usage: Usage, tool_rounds: u32 },
    SessionComplete,
    Error(String),
}
```

- [ ] **Step 4: 验证编译 + 提交**

Run: `cargo check -p dshns-core`
```bash
git add crates/core/src/{config, error, event}.rs && git commit -m "feat(core): add Config, Error, Event types"
```

---

### Task 4: deepseek-client (request, response, client)

**Files:** `crates/deepseek-client/src/request.rs`, `response.rs`, `client.rs`

- [ ] **Step 1: 写 request.rs**

```rust
use serde::Serialize;
use dshns_core::message::Message;
use dshns_core::tool::ToolDef;

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}
```

- [ ] **Step 2: 写 response.rs**

```rust
use serde::Deserialize;
use dshns_core::message::{Usage, ToolCallRef};

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ResponseMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseMessage {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallRef>>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta { delta: String },
    ToolCallStart { id: String, name: String },
    ToolCallArgDelta { id: String, delta: String },
    ToolCallComplete { id: String, name: String, arguments: serde_json::Value },
    Finished { finish_reason: String, usage: Usage },
    Error { message: String },
}

/// 解析一行 SSE data
pub fn parse_sse_line(line: &str) -> Option<StreamEvent> {
    let line = line.trim();
    if line.is_empty() { return None; }
    if line == "data: [DONE]" {
        return Some(StreamEvent::Finished {
            finish_reason: "stop".into(),
            usage: Usage::default(),
        });
    }
    if !line.starts_with("data: ") { return None; }
    let json_str = &line[6..];
    let chunk: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let choices = chunk["choices"].as_array()?;

    if choices.is_empty() {
        let reason = chunk["choices"][0]["finish_reason"].as_str().unwrap_or("stop").to_string();
        let usage = serde_json::from_value(chunk["usage"].clone()).unwrap_or_default();
        return Some(StreamEvent::Finished { finish_reason: reason, usage });
    }

    let delta = &choices[0]["delta"];

    if let Some(tool_calls) = delta["tool_calls"].as_array() {
        for tc in tool_calls {
            if let (Some(id), Some(name)) = (tc["id"].as_str(), tc["function"]["name"].as_str()) {
                return Some(StreamEvent::ToolCallStart { id: id.into(), name: name.into() });
            }
            if let Some(args) = tc["function"]["arguments"].as_str() {
                if !args.is_empty() {
                    return Some(StreamEvent::ToolCallArgDelta {
                        id: tc["index"].as_u64().map(|i| format!("pending_{}", i)).unwrap_or_default(),
                        delta: args.into(),
                    });
                }
            }
        }
    }

    if let Some(content) = delta["content"].as_str() {
        if !content.is_empty() {
            return Some(StreamEvent::TextDelta { delta: content.into() });
        }
    }

    if let Some(reason) = choices[0]["finish_reason"].as_str() {
        if reason != "null" {
            let usage = serde_json::from_value(chunk["usage"].clone()).unwrap_or_default();
            return Some(StreamEvent::Finished { finish_reason: reason.into(), usage });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_delta() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#;
        match parse_sse_line(line) {
            Some(StreamEvent::TextDelta { delta }) => assert_eq!(delta, "Hello"),
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_call_start() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":""}}]}}]}"#;
        match parse_sse_line(line) {
            Some(StreamEvent::ToolCallStart { id, name }) => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
            }
            other => panic!("expected ToolCallStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_done() {
        match parse_sse_line("data: [DONE]") {
            Some(StreamEvent::Finished { .. }) => {},
            other => panic!("expected Finished, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_empty() {
        assert!(parse_sse_line("").is_none());
    }
}
```

Run: `cargo test -p dshns-deepseek-client`
Expected: 4 tests PASS

- [ ] **Step 3: 写 client.rs**

```rust
use std::time::Duration;
use reqwest::Client as HttpClient;
use dshns_core::config::ApiConfig;
use dshns_core::error::DshnsError;
use crate::request::ChatRequest;
use crate::response::{ChatResponse, StreamEvent, parse_sse_line};
use futures::Stream;
use std::pin::Pin;
use tokio::sync::mpsc;

pub struct DeepSeekClient {
    http: HttpClient,
    api_key: String,
    base_url: String,
    config: ApiConfig,
}

impl DeepSeekClient {
    pub fn new(api_key: String, config: ApiConfig) -> Result<Self, DshnsError> {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|e| DshnsError::Network(e.to_string()))?;
        Ok(Self { http, api_key, base_url: "https://api.deepseek.com".into(), config })
    }

    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, DshnsError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut r = req.clone();
        r.stream = false;
        let resp = self.http.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&r).send().await
            .map_err(|e| self.classify_error(e))?;
        self.handle_response(resp).await
    }

    pub async fn chat_stream(&self, req: &ChatRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, DshnsError>> + Send>>, DshnsError>
    {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut r = req.clone();
        r.stream = true;
        let resp = self.http.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&r).send().await
            .map_err(|e| self.classify_error(e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_http_error(status.as_u16(), &body));
        }

        let stream = resp.bytes_stream();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            use futures::StreamExt;
            tokio::pin!(stream);
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(b) => {
                        buf.push_str(&String::from_utf8_lossy(&b));
                        while let Some(pos) = buf.find('\n') {
                            let line = buf[..pos].to_string();
                            buf = buf[pos+1..].to_string();
                            if let Some(ev) = parse_sse_line(&line) {
                                if tx.send(Ok(ev)).is_err() { return; }
                            }
                        }
                    }
                    Err(e) => { let _ = tx.send(Err(DshnsError::Network(e.to_string()))); return; }
                }
            }
        });
        Ok(Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx)))
    }

    async fn handle_response(&self, resp: reqwest::Response) -> Result<ChatResponse, DshnsError> {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(self.classify_http_error(status.as_u16(), &body));
        }
        serde_json::from_str(&body).map_err(|e| DshnsError::SseParse(e.to_string()))
    }

    fn classify_http_error(&self, status: u16, body: &str) -> DshnsError {
        match status {
            401 => DshnsError::ApiAuth(body.into()),
            429 => DshnsError::ApiRateLimited(5),
            500..=599 => DshnsError::ApiServer(body.into()),
            _ => DshnsError::Other(format!("HTTP {}: {}", status, body)),
        }
    }

    fn classify_error(&self, e: reqwest::Error) -> DshnsError {
        if e.is_timeout() {
            DshnsError::Network("请求超时".into())
        } else if e.is_connect() {
            DshnsError::Network(format!("连接失败: {}", e))
        } else {
            DshnsError::Network(e.to_string())
        }
    }
}
```

- [ ] **Step 4: 验证编译 + 提交**

Run: `cargo test -p dshns-deepseek-client`
```bash
git add crates/deepseek-client/ && git commit -m "feat(deepseek-client): SSE streaming client + parser"
```

---

### Task 5: tools — Registry + 4 Builtin Tools + Executor

**Files:** `crates/tools/src/registry.rs`, `executor.rs`, `builtin/read_file.rs`, `write_file.rs`, `exec_shell.rs`, `search_code.rs`

- [ ] **Step 1: 写 registry.rs**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use dshns_core::tool::{Tool, ToolDef};

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: HashMap::new() } }
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().function.name.clone();
        self.tools.insert(name, tool);
    }
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> { self.tools.get(name).cloned() }
    pub fn get_names(&self) -> Vec<String> { self.tools.keys().cloned().collect() }
    pub fn to_api_tools(&self) -> Vec<ToolDef> { self.tools.values().map(|t| t.definition()).collect() }
    pub fn to_api_tools_excluding(&self, exclude: &[&str]) -> Vec<ToolDef> {
        self.tools.values()
            .filter(|t| !exclude.contains(&t.definition().function.name.as_str()))
            .map(|t| t.definition()).collect()
    }
}
```

- [ ] **Step 2: 写 read_file.rs**

```rust
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
```

- [ ] **Step 3: 写 write_file.rs**

```rust
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
```

- [ ] **Step 4: 写 exec_shell.rs**

```rust
use std::{collections::HashMap, time::Duration};
use async_trait::async_trait;
use dshns_core::tool::*;
use regex::Regex;

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
```

- [ ] **Step 5: 写 search_code.rs**

```rust
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
```

- [ ] **Step 6: 写 executor.rs + builtin/mod.rs**

`executor.rs`:
```rust
use std::sync::Arc;
use std::time::Duration;
use dshns_core::tool::{ToolCall, ToolOutcome, ToolStatus};
use crate::registry::ToolRegistry;

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    default_timeout: Duration,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, timeout_secs: u64) -> Self {
        Self { registry, default_timeout: Duration::from_secs(timeout_secs) }
    }

    pub async fn exec_one(&self, call: &ToolCall) -> ToolOutcome {
        let tool = match self.registry.get(&call.name) {
            Some(t) => t,
            None => return ToolOutcome {
                call_id: call.id.clone(),
                status: ToolStatus::Error { reason: format!("未知工具: {}", call.name) },
                content: String::new(), was_truncated: false,
            },
        };
        match tokio::time::timeout(self.default_timeout, tool.execute(call)).await {
            Ok(outcome) => outcome,
            Err(_) => ToolOutcome { call_id: call.id.clone(), status: ToolStatus::Timeout, content: format!("工具 {} 执行超时", call.name), was_truncated: false },
        }
    }

    pub async fn exec_many(&self, calls: &[ToolCall]) -> Vec<ToolOutcome> {
        futures::future::join_all(calls.iter().map(|c| self.exec_one(c))).await
    }
}
```

- [ ] **Step 7: 验证编译 + 测试 + 提交**

Run: `cargo test -p dshns-tools -- --test-threads=1`
```bash
git add crates/tools/ && git commit -m "feat(tools): registry + 4 builtin tools + executor"
```

---

### Task 6: session-store

**Files:** `crates/session-store/src/store.rs`, `memory.rs`, `prompt.rs`

- [ ] **Step 1: 写 store.rs**

```rust
use std::path::PathBuf;
use dshns_core::message::Message;
use dshns_core::session::{Session, SessionId, SessionMeta};
use dshns_core::error::DshnsError;

pub struct SessionStore { root: PathBuf }

impl SessionStore {
    pub fn new() -> Result<Self, DshnsError> {
        let home = home_dir()?;
        let root = home.join(".dsHns_rs/sessions");
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn session_dir(&self, id: &SessionId) -> PathBuf { self.root.join(id.to_dir_name()) }

    pub fn create(&self, working_dir: &std::path::Path) -> Result<SessionId, DshnsError> {
        let meta = SessionMeta::new(working_dir.to_path_buf());
        let dir = self.session_dir(&meta.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap())?;
        std::fs::write(dir.join("messages.jsonl"), "")?;
        Ok(meta.id)
    }

    pub fn append_message(&self, id: &SessionId, msg: &Message) -> Result<(), DshnsError> {
        let path = self.session_dir(id).join("messages.jsonl");
        let mut line = serde_json::to_string(msg).unwrap();
        line.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(line.as_bytes())?;
        Ok(())
    }

    pub fn append_messages(&self, id: &SessionId, msgs: &[Message]) -> Result<(), DshnsError> {
        for m in msgs { self.append_message(id, m)?; }
        Ok(())
    }

    pub fn load(&self, id: &SessionId) -> Result<Session, DshnsError> {
        let dir = self.session_dir(id);
        if !dir.exists() { return Err(DshnsError::SessionNotFound(id.to_string())); }
        let meta: SessionMeta = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?)?;
        let content = std::fs::read_to_string(dir.join("messages.jsonl"))?;
        let messages: Vec<Message> = content.lines().filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok()).collect();
        Ok(Session { meta, messages })
    }

    pub fn list(&self) -> Result<Vec<SessionMeta>, DshnsError> {
        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let mp = entry.path().join("meta.json");
                if mp.exists() {
                    if let Ok(meta) = serde_json::from_str::<SessionMeta>(&std::fs::read_to_string(&mp)?) {
                        sessions.push(meta);
                    }
                }
            }
        }
        sessions.sort_by_key(|s| s.updated_at);
        sessions.reverse();
        Ok(sessions)
    }
}

fn home_dir() -> Result<PathBuf, DshnsError> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from).map_err(|_| DshnsError::Config("无法获取 HOME".into()))
}
```

- [ ] **Step 2: 写 prompt.rs + memory.rs + 提交**

```rust
// prompt.rs
use std::path::PathBuf;
use dshns_core::error::DshnsError;

const DEFAULT_AGENTS_MD: &str = r#"## 环境
- 操作系统：Windows
- Shell：PowerShell（powershell.exe -NoProfile）
- 编码：UTF-8（代码页 65001）
- 文件写入编码：UTF-8 无 BOM

## 安全限制（不可违反）
- 禁止使用 Remove-Item -Recurse -Force、del /f /s 等递归强制删除
- 需要删除文件时，必须逐个指定文件路径
- 禁止使用 runas 等提权命令
- 禁止对系统目录进行写操作
"#;

pub struct PromptLoader;

impl PromptLoader {
    pub fn load(working_dir: &std::path::Path) -> Result<String, DshnsError> {
        let mut parts = vec![Self::load_or_create_global()?];
        let local = working_dir.join("AGENTS.md");
        if local.exists() {
            parts.push(std::fs::read_to_string(&local)?);
        }
        Ok(parts.join("\n\n"))
    }

    fn load_or_create_global() -> Result<String, DshnsError> {
        let home = home_dir()?;
        let dir = home.join(".dsHns_rs");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("AGENTS.md");
        if !path.exists() {
            std::fs::write(&path, DEFAULT_AGENTS_MD)?;
            eprintln!("已创建默认全局提示词: {}", path.display());
        }
        Ok(std::fs::read_to_string(&path)?)
    }
}

fn home_dir() -> Result<PathBuf, DshnsError> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from).map_err(|_| DshnsError::Config("无法获取 HOME".into()))
}
```

```rust
// memory.rs
use dshns_core::error::DshnsError;

pub struct MemoryStore { root: std::path::PathBuf }

impl MemoryStore {
    pub fn new() -> Result<Self, DshnsError> {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
            .map(std::path::PathBuf::from)
            .map_err(|_| DshnsError::Config("无法获取 HOME".into()))?;
        let root = home.join(".dsHns_rs/memory");
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn list(&self) -> Result<Vec<String>, DshnsError> {
        Ok(std::fs::read_dir(&self.root)?.filter_map(|e| {
            e.ok().and_then(|e| e.file_name().to_str().map(|s| s.to_string()))
        }).filter(|n| n.ends_with(".md")).collect())
    }
}
```

Run: `cargo check -p dshns-session-store`
```bash
git add crates/session-store/ && git commit -m "feat(session-store): SessionStore + PromptLoader + MemoryStore"
```

---

### Task 7: agent — Safety, Approval, Context, AgentLoop, SubAgent

**Files:** `crates/agent/src/safety.rs`, `approval.rs`, `context.rs`, `loop.rs`, `subagent.rs`

- [ ] **Step 1: 写 safety.rs**

```rust
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
```

- [ ] **Step 2: 写 approval.rs**

```rust
use dshns_core::config::ApprovalMode;
use dshns_core::tool::ToolCall;
use regex::Regex;

pub struct Approver { mode: ApprovalMode, danger: Vec<Regex> }

pub enum ApprovalVerdict { Allow, NeedsConfirmation { reason: String } }

impl Approver {
    pub fn new(mode: ApprovalMode) -> Self {
        let danger = vec![
            r"(?i)\bremove-item\b", r"(?i)\bdel\b\s",
            r"C:\\Windows", r"C:\\Program Files",
            r"(?i)invoke-webrequest.*\|.*invoke-expression",
        ].into_iter().map(|p| Regex::new(p).unwrap()).collect();
        Self { mode, danger }
    }

    pub fn set_mode(&mut self, m: ApprovalMode) { self.mode = m; }
    pub fn mode(&self) -> ApprovalMode { self.mode }

    pub fn check(&self, call: &ToolCall) -> ApprovalVerdict {
        match self.mode {
            ApprovalMode::Auto => ApprovalVerdict::Allow,
            ApprovalMode::Paranoid => ApprovalVerdict::NeedsConfirmation { reason: format!("确认执行 {}？", call.name) },
            ApprovalMode::Confirm => {
                if call.name == "exec_shell" {
                    if let Some(cmd) = call.arguments["cmd"].as_str() {
                        for p in &self.danger {
                            if p.is_match(cmd) { return ApprovalVerdict::NeedsConfirmation { reason: format!("危险命令: {}", cmd) }; }
                        }
                    }
                }
                ApprovalVerdict::Allow
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; use serde_json::json;
    fn mk_call(name: &str, cmd: &str) -> ToolCall { ToolCall { id: "1".into(), name: name.into(), arguments: json!({"cmd": cmd}) } }
    #[test] fn test_auto() { assert!(matches!(Approver::new(ApprovalMode::Auto).check(&mk_call("exec_shell","rm x")), ApprovalVerdict::Allow)); }
    #[test] fn test_paranoid() { assert!(matches!(Approver::new(ApprovalMode::Paranoid).check(&mk_call("read_file","")), ApprovalVerdict::NeedsConfirmation{..})); }
    #[test] fn test_confirm_danger() { assert!(matches!(Approver::new(ApprovalMode::Confirm).check(&mk_call("exec_shell","Remove-Item x")), ApprovalVerdict::NeedsConfirmation{..})); }
}
```

- [ ] **Step 3: 写 context.rs**

```rust
use dshns_core::message::Message;
use dshns_core::config::ContextConfig;

pub struct ContextManager { config: ContextConfig }

impl ContextManager {
    pub fn new(config: ContextConfig) -> Self { Self { config } }

    pub fn build_messages(&self, system_prompt: &str, history: &[Message], input: &str) -> Vec<Message> {
        let mut msgs = vec![Message::System { content: system_prompt.into() }];
        msgs.extend(history.iter().cloned());
        msgs.push(Message::User { content: input.into() });
        let estimated = self.estimate(&msgs);
        let threshold = (self.config.max_window_tokens as f64 * self.config.compression_threshold) as usize;
        if estimated > threshold { msgs = self.compress(msgs); }
        msgs
    }

    fn estimate(&self, msgs: &[Message]) -> usize {
        msgs.iter().map(|m| match m {
            Message::System { content } | Message::User { content } => content.len(),
            Message::Assistant { content, tool_calls } => {
                content.as_ref().map(|c| c.len()).unwrap_or(0)
                + tool_calls.as_ref().map(|v| v.iter().map(|t| t.function.arguments.len()).sum::<usize>()).unwrap_or(0)
            }
            Message::Tool { content, .. } => content.len(),
        }).sum::<usize>() / 3
    }

    fn compress(&self, msgs: Vec<Message>) -> Vec<Message> {
        let system = msgs.first().cloned();
        let mut result = vec![system.unwrap()];
        for (i, msg) in msgs.into_iter().skip(1).enumerate() {
            match &msg {
                Message::Tool { content, .. } if i < msgs.len().saturating_sub(7) => {
                    result.push(Message::Tool { tool_call_id: "truncated".into(), content: self.truncate(content, self.config.max_tool_result_tokens) });
                }
                _ => result.push(msg),
            }
        }
        result
    }

    pub fn truncate(&self, content: &str, max_tokens: usize) -> String {
        let max_chars = max_tokens * 3;
        if content.len() <= max_chars { return content.into(); }
        let head: String = content.chars().take(max_chars * 4 / 10).collect();
        let tail: String = content.chars().rev().take(max_chars * 5 / 10).collect::<String>().chars().rev().collect();
        format!("{}\n\n[... {} 字符已省略 ...]\n\n{}", head, content.len() - head.len() - tail.len(), tail)
    }
}
```

- [ ] **Step 4: 写 loop.rs (AgentLoop 核心)**

```rust
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;
use dshns_core::{
    config::{AgentConfig, ApprovalMode, ContextConfig},
    event::AgentEvent, message::{Message, Usage},
    tool::{ToolCall, ToolOutcome, ToolStatus}, error::DshnsError,
};
use dshns_deepseek_client::client::DeepSeekClient;
use dshns_deepseek_client::request::ChatRequest;
use dshns_deepseek_client::response::StreamEvent;
use dshns_tools::registry::ToolRegistry;
use dshns_tools::executor::ToolExecutor;
use crate::{context::ContextManager, safety::SafetyGuard, approval::{Approver, ApprovalVerdict}};

pub struct AgentLoop {
    client: Arc<DeepSeekClient>,
    executor: Arc<ToolExecutor>,
    registry: Arc<ToolRegistry>,
    config: AgentConfig,
    context_manager: ContextManager,
    safety_guard: SafetyGuard,
    approver: std::sync::Mutex<Approver>,
    pub depth: u8,
    allowed_tools: Option<Vec<String>>,
    system_prompt: String,
}

#[derive(Debug)]
pub struct AgentOutcome {
    pub final_response: String,
    pub messages: Vec<Message>,
    pub usage: Usage,
    pub tool_rounds: u32,
}

impl AgentLoop {
    pub fn new(
        client: Arc<DeepSeekClient>, executor: Arc<ToolExecutor>,
        registry: Arc<ToolRegistry>, config: AgentConfig,
        context_config: ContextConfig, approval_mode: ApprovalMode,
        depth: u8, system_prompt: String,
    ) -> Self {
        Self {
            client, executor, registry, config,
            context_manager: ContextManager::new(context_config),
            safety_guard: SafetyGuard::new(),
            approver: std::sync::Mutex::new(Approver::new(approval_mode)),
            depth, allowed_tools: None, system_prompt,
        }
    }

    pub fn set_allowed_tools(&mut self, tools: Vec<String>) { self.allowed_tools = Some(tools); }
    pub fn set_approval_mode(&self, mode: ApprovalMode) { self.approver.lock().unwrap().set_mode(mode); }

    pub async fn run(&self, user_input: &str, history: Vec<Message>, event_tx: mpsc::UnboundedSender<AgentEvent>) -> Result<AgentOutcome, DshnsError> {
        let _ = event_tx.send(AgentEvent::UserInput(user_input.into()));
        let mut messages = self.context_manager.build_messages(&self.system_prompt, &history, user_input);
        let mut tool_rounds: u32 = 0;
        let mut total_usage = Usage::default();
        let mut consecutive_failures: u32 = 0;

        loop {
            let api_tools = if let Some(ref allowed) = self.allowed_tools {
                let exclude: Vec<&str> = self.registry.get_names().iter()
                    .filter(|n| !allowed.contains(n)).map(|s| s.as_str()).collect();
                self.registry.to_api_tools_excluding(&exclude)
            } else {
                self.registry.to_api_tools()
            };

            let req = ChatRequest {
                model: "deepseek-v4-flash".into(), messages: messages.clone(),
                tools: Some(api_tools), stream: true,
                temperature: Some(0.0), max_tokens: Some(8192),
            };

            let mut stream = self.client.chat_stream(&req).await?;
            use futures::StreamExt;
            let mut text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut pending: HashMap<String, (String, String)> = HashMap::new();
            let mut usage = Usage::default();

            while let Some(ev) = stream.next().await {
                match ev {
                    Ok(StreamEvent::TextDelta { delta }) => { text.push_str(&delta); let _ = event_tx.send(AgentEvent::Thinking(delta)); }
                    Ok(StreamEvent::ToolCallStart { id, name }) => { pending.insert(id, (name, String::new())); }
                    Ok(StreamEvent::ToolCallArgDelta { id, delta }) => { if let Some((_, a)) = pending.get_mut(&id) { a.push_str(&delta); } }
                    Ok(StreamEvent::ToolCallComplete { id, name, arguments }) => { tool_calls.push(ToolCall { id, name, arguments }); pending.remove(&id); }
                    Ok(StreamEvent::Finished { usage: u, .. }) => { usage = u; break; }
                    Ok(StreamEvent::Error { message }) => return Err(DshnsError::SseParse(message)),
                    Err(e) => return Err(e),
                }
            }

            // 处理未完成的 tool calls
            for (id, (name, args)) in pending.drain() {
                if let Ok(args_val) = serde_json::from_str(&args) {
                    tool_calls.push(ToolCall { id, name, arguments: args_val });
                }
            }

            total_usage.prompt_tokens += usage.prompt_tokens;
            total_usage.completion_tokens += usage.completion_tokens;
            total_usage.total_tokens += usage.total_tokens;

            // 追加 assistant message
            if tool_calls.is_empty() {
                messages.push(Message::Assistant { content: Some(text.clone()), tool_calls: None });
            } else {
                let refs: Vec<_> = tool_calls.iter().map(|tc| dshns_core::message::ToolCallRef {
                    id: tc.id.clone(), call_type: "function".into(),
                    function: dshns_core::message::FunctionRef { name: tc.name.clone(), arguments: tc.arguments.to_string() },
                }).collect();
                messages.push(Message::Assistant { content: if text.is_empty() { None } else { Some(text) }, tool_calls: Some(refs) });
            }

            if tool_calls.is_empty() {
                let _ = event_tx.send(AgentEvent::TurnComplete { usage: total_usage.clone(), tool_rounds });
                let _ = event_tx.send(AgentEvent::SessionComplete);
                return Ok(AgentOutcome { final_response: text, messages, usage: total_usage, tool_rounds });
            }

            tool_rounds += 1;
            if tool_rounds > self.config.max_tool_rounds {
                let _ = event_tx.send(AgentEvent::Error("达到最大工具轮数".into()));
                let _ = event_tx.send(AgentEvent::SessionComplete);
                return Ok(AgentOutcome { final_response: text, messages, usage: total_usage, tool_rounds });
            }

            for call in &tool_calls {
                // 硬限制
                if let Some(blocked) = self.safety_guard.check(call) {
                    let _ = event_tx.send(AgentEvent::ToolBlocked { call: call.clone(), reason: format!("{:?}", blocked.status) });
                    messages.push(Message::Tool { tool_call_id: call.id.clone(), content: blocked.content.clone() });
                    continue;
                }
                // 软审批
                match self.approver.lock().unwrap().check(call) {
                    ApprovalVerdict::NeedsConfirmation { reason } => {
                        let _ = event_tx.send(AgentEvent::ToolConfirmationNeeded { call: call.clone(), reason });
                        // TODO: 等待用户 y/n — MVP 暂时跳过
                        continue;
                    }
                    ApprovalVerdict::Allow => {}
                }
                // 执行
                let _ = event_tx.send(AgentEvent::ToolCallStart { id: call.id.clone(), name: call.name.clone() });
                let outcome = self.executor.exec_one(call).await;
                let _ = event_tx.send(AgentEvent::ToolExecution { call_id: call.id.clone(), status: outcome.status.clone(), summary: outcome.content.chars().take(200).collect() });

                match &outcome.status {
                    ToolStatus::Error { .. } | ToolStatus::Timeout => consecutive_failures += 1,
                    ToolStatus::HardBlocked { .. } => {},
                    _ => consecutive_failures = 0,
                }
                if consecutive_failures >= 5 {
                    return Err(DshnsError::ToolLoopStuck { failures: consecutive_failures });
                }
                messages.push(Message::Tool { tool_call_id: call.id.clone(), content: self.format_result(&outcome, call) });
            }
        }
    }

    fn format_result(&self, outcome: &ToolOutcome, call: &ToolCall) -> String {
        match &outcome.status {
            ToolStatus::Success => format!("工具 {} 执行成功:\n{}", call.name, outcome.content),
            ToolStatus::Error { reason } => format!("工具 {} 执行失败: {}", call.name, reason),
            ToolStatus::Timeout => format!("工具 {} 超时，请尝试其他方式", call.name),
            ToolStatus::Denied => "用户拒绝了此操作".into(),
            ToolStatus::HardBlocked { reason } => format!("{} 已被系统禁止: {}", call.name, reason),
        }
    }
}
```

- [ ] **Step 5: 写 subagent.rs (MVP stubs)**

```rust
use std::collections::HashMap;
use std::sync::Arc;
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
```

- [ ] **Step 6: 验证编译 + 测试 + 提交**

Run: `cargo test -p dshns-agent`
```bash
git add crates/agent/ && git commit -m "feat(agent): SafetyGuard + Approver + Context + AgentLoop + SubAgent stubs"
```

---

### Task 8: app — CLI + REPL + main

**Files:** `crates/app/src/cli.rs`, `repl.rs`, `main.rs`

- [ ] **Step 1: 写 cli.rs**

```rust
use std::path::PathBuf;
use clap::Parser;

#[derive(Parser)]
#[command(name = "dshns", about = "DeepSeek Agent", version = "0.1.0")]
pub struct Cli {
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,
    #[arg(short = 'c', long = "continue")]
    pub resume: bool,
    #[arg(short = 'm', long = "model")]
    pub model: Option<String>,
    #[arg(short = 'd', long = "dir")]
    pub working_dir: Option<PathBuf>,
    #[arg(long = "sessions")]
    pub list_sessions: bool,
    #[arg(long = "resume-session")]
    pub resume_session: Option<String>,
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}
```

- [ ] **Step 2: 写 repl.rs**

```rust
use std::sync::Arc;
use tokio::sync::mpsc;
use dshns_core::event::AgentEvent;
use dshns_core::tool::ToolStatus;
use dshns_agent::loop::AgentLoop;
use rustyline::{Editor, error::ReadlineError};

pub struct Repl { editor: Editor<()>, agent: Arc<AgentLoop> }

impl Repl {
    pub fn new(agent: Arc<AgentLoop>) -> Self {
        let mut editor = Editor::<()>::new().unwrap();
        let _ = editor.load_history(history_path().as_deref().unwrap_or(""));
        Self { editor, agent }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("dsHns DeepSeek Agent — /help 帮助, /exit 退出");
        loop {
            let line = match self.editor.readline("\n> ") {
                Ok(l) => l.trim().to_string(),
                Err(ReadlineError::Interrupted) => { println!("\n已中断"); continue; }
                Err(ReadlineError::Eof) => { println!("\n再见!"); break; }
                Err(e) => { eprintln!("错误: {}", e); break; }
            };
            if line.is_empty() { continue; }
            self.editor.add_history_entry(&line)?;

            if let Some(cmd) = line.strip_prefix('/') {
                match cmd {
                    "exit" | "e" | "quit" | "q" => break,
                    "help" | "h" => Self::help(),
                    "clear" => print!("\x1B[2J\x1B[1;1H"),
                    _ => self.process(&line).await?,
                }
            } else {
                self.process(&line).await?;
            }
        }
        let _ = self.editor.save_history(history_path().as_deref().unwrap_or(""));
        Ok(())
    }

    async fn process(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let agent = self.agent.clone();
        let owned = input.to_string();
        tokio::spawn(async move { agent.run(&owned, vec![], tx).await });

        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Thinking(d) => { print!("{}", d); use std::io::Write; std::io::stdout().flush().ok(); }
                AgentEvent::ToolCallStart { name, .. } => println!("\n  🔧 {}", name),
                AgentEvent::ToolBlocked { reason, .. } => println!("\n  🚫 {}", reason),
                AgentEvent::ToolExecution { status, summary, .. } => match status {
                    ToolStatus::Success => println!("  ✓ {}", summary),
                    ToolStatus::Error { reason } => println!("  ✗ {}", reason),
                    _ => println!("  ℹ {:?}", status),
                },
                AgentEvent::TurnComplete { usage, tool_rounds } => println!("\n  [{} tokens | {} 轮工具]", usage.total_tokens, tool_rounds),
                AgentEvent::Error(m) => eprintln!("\n  ✗ {}", m),
                AgentEvent::SessionComplete => break,
                _ => {}
            }
        }
        println!();
        Ok(())
    }

    fn help() {
        println!("/help, /h  帮助  /exit, /e  退出  /clear  清屏  Ctrl+C  中断  输入内容  发给 AI");
    }
}

fn history_path() -> Option<String> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    Some(format!("{}/.dsHns_rs/.history", home))
}
```

- [ ] **Step 3: 写 main.rs**

```rust
mod cli; mod repl;

use std::sync::Arc;
use clap::Parser;
use dshns_core::config::{AppConfig, ApprovalMode};
use dshns_core::error::DshnsError;
use dshns_deepseek_client::client::DeepSeekClient;
use dshns_tools::registry::ToolRegistry;
use dshns_tools::executor::ToolExecutor;
use dshns_tools::builtin::{read_file::ReadFileTool, write_file::WriteFileTool, exec_shell::ExecShellTool, search_code::SearchCodeTool};
use dshns_session_store::store::SessionStore;
use dshns_session_store::prompt::PromptLoader;
use dshns_session_store::memory::MemoryStore;
use dshns_agent::loop::AgentLoop;
use dshns_agent::subagent::{AgentOpenTool, AgentCloseTool, AgentResultTool};

fn load_config() -> Result<AppConfig, DshnsError> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from).map_err(|_| DshnsError::Config("HOME not found".into()))?;
    let dir = home.join(".dsHns_rs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.toml");
    if path.exists() {
        let s = std::fs::read_to_string(&path).map_err(|e| DshnsError::Config(e.to_string()))?;
        toml::from_str(&s).map_err(|e| DshnsError::Config(e.to_string()))
    } else {
        let cfg = AppConfig::default();
        std::fs::write(&path, toml::to_string_pretty(&cfg).unwrap())?;
        eprintln!("已创建默认配置: {}", path.display());
        Ok(cfg)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = cli::Cli::parse();
    let mut config = load_config()?;
    if let Some(ref m) = cli.model { config.api.model = m.clone(); }

    let api_key = std::env::var("DEEPSEEK_API_KEY").map_err(|_| "请设置 DEEPSEEK_API_KEY 环境变量")?;
    let working_dir = cli.working_dir.unwrap_or_else(|| std::env::current_dir().unwrap());
    let system_prompt = PromptLoader::load(&working_dir)?;

    let client = Arc::new(DeepSeekClient::new(api_key, config.api.clone())?);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(ExecShellTool::new(config.agent.tool_timeout_secs)));
    registry.register(Arc::new(SearchCodeTool));
    registry.register(Arc::new(AgentOpenTool));
    registry.register(Arc::new(AgentCloseTool));
    registry.register(Arc::new(AgentResultTool));

    let registry = Arc::new(registry);
    let executor = Arc::new(ToolExecutor::new(registry.clone(), config.agent.tool_timeout_secs));
    let mode = ApprovalMode::from_str(&config.mode.default);

    let agent = Arc::new(AgentLoop::new(
        client.clone(), executor.clone(), registry.clone(),
        config.agent.clone(), config.context.clone(), mode, 1, system_prompt,
    ));

    if cli.list_sessions {
        let store = SessionStore::new()?;
        for s in store.list()? {
            println!("{}  {}  ({}msgs)  {}",
                s.id.to_string().chars().take(8).collect::<String>(),
                s.title, s.message_count,
                s.updated_at.format("%Y-%m-%d %H:%M"));
        }
        return Ok(());
    }

    if let Some(prompt) = cli.prompt {
        println!("处理中...");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let a = agent.clone();
        tokio::spawn(async move { a.run(&prompt, vec![], tx).await });
        while let Some(ev) = rx.recv().await {
            match ev {
                dshns_core::event::AgentEvent::Thinking(d) => print!("{}", d),
                dshns_core::event::AgentEvent::SessionComplete => break,
                _ => {}
            }
        }
        println!();
    } else {
        repl::Repl::new(agent).run().await?;
    }

    Ok(())
}
```

- [ ] **Step 4: 验证编译 + 修复 + 提交**

Run: `cargo check`
Expected: 全 workspace 编译通过（可能会有一些 import/warning 需要修复）

```bash
# 修复所有编译错误后
git add crates/app/ && git commit -m "feat(app): CLI + REPL + main startup wiring"
```

---

### Task 9: 端到端集成测试

**Files:** `tests/integration_test.rs`

- [ ] **Step 1: 写测试骨架**

```rust
#[cfg(test)]
mod integration {
    #[test]
    #[ignore]
    fn test_simple_chat() {
        // DEEPSEEK_API_KEY=sk-xxx cargo test --test integration_test -- --ignored
        todo!("设置 API Key 后运行实际对话测试")
    }

    #[test]
    #[ignore]
    fn test_tool_call_roundtrip() {
        // 测试: "读取 Cargo.toml" → read_file → 返回内容 → 模型总结
        todo!("设置 API Key 后运行工具调用测试")
    }

    #[test]
    #[ignore]
    fn test_safety_block_rm_rf() {
        // 验证 exec_shell 的硬限制拦截
    }
}
```

- [ ] **Step 2: 最终全量检查**

Run: `cargo check && cargo test`
Expected: 所有编译通过，所有单元测试 PASS，集成测试 skipped (ignored)

- [ ] **Step 3: 功能验证**

```bash
# 设置 Key 后测试
cargo run -- -p "你好，1+1等于几？"
# Expected: 流式输出文本回复

cargo run -- -p "读取当前目录的 Cargo.toml 文件" --dir .
# Expected: 调用 read_file，返回内容后模型回复
```

- [ ] **Step 4: 最终提交**

```bash
git add -A && git commit -m "feat: complete MVP — agent loop + tools + REPL"
```
