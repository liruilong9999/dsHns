# dsHns — DeepSeek 编程助手

基于 DeepSeek API 的命令行 AI 编程助手，架构类似 Claude Code。

## 技术栈

- Rust 2021 edition
- tokio (异步运行时)
- reqwest (HTTP 客户端)
- rustyline (REPL)
- clap (CLI 参数)
- DeepSeek API (OpenAI 兼容格式，SSE 流式)

## 快速开始

```bash
# 设置 API Key
set DEEPSEEK_API_KEY=sk-xxx

# 编译
cargo build --release

# REPL 交互模式
cargo run

# 一次性模式
cargo run -- -p "帮我修个 bug"

# 运行测试
cargo test
```

## 架构

### 事件驱动管道

```
用户输入 → CLI/REPL → AgentLoop → DeepSeek API (SSE)
                         ↑              ↓
                         │     AgentEvent 流
                         ↓
                   SafetyGuard → Approver → ToolExecutor
```

### Cargo Workspace (6 crates)

```
crates/
├── core/               # 领域模型 (Message, Tool, Session, Config, Event, Error)
├── deepseek-client/    # DeepSeek API 客户端 (SSE 解析, HTTP 请求)
├── tools/              # 工具系统 (Registry, Executor, 4 builtin tools)
├── session-store/      # 会话持久化 (JSONL), 提示词加载 (AGENTS.md)
├── agent/              # Agent 循环, 安全检查, 子智能体
└── app/                # CLI 入口, REPL
```

**依赖方向**: `app → agent → {deepseek-client, tools, session-store} → core`
禁止循环依赖，`core` 是唯一共享依赖。

### 安全双层检查

1. **SafetyGuard (硬限制)**: 不可配置、不可绕过。拦截 `Remove-Item -Recurse -Force`、`del /f /s /q`、`runas`、`format`、`diskpart`。不弹审批，直接返回错误给模型。
2. **Approver (软审批)**: 三级模式 (auto/confirm/paranoid)，可通过 `/mode` 运行时切换。

### 配置

- 配置目录: `~/.dsHns_rs/`
- 配置文件: `settings.toml` (首次运行自动创建默认值)
- 全局提示词: `~/.dsHns_rs/AGENTS.md`
- 本地提示词: `./AGENTS.md` (可选，追加到全局之后)
- 会话目录: `~/.dsHns_rs/sessions/<uuid>/` (meta.json + messages.jsonl)
- 默认模型: `deepseek-v4-flash`

### Shell 环境

- PowerShell: `powershell.exe -NoProfile -Command "chcp 65001 > $null; <cmd>"`
- 文件编码: UTF-8 无 BOM

### 关键常量

| 配置项 | 默认值 |
|--------|--------|
| max_tool_rounds | 25 |
| tool_timeout_secs | 60 |
| context window | 131072 tokens |
| compression_threshold | 75% |
| subagent max_rounds | 10 |
| subagent timeout | 300s |

## 目录结构

```
dsHns/
├── Cargo.toml              # workspace root
├── .gitignore
├── CLAUDE.md               # 本文件
├── docs/
│   └── superpowers/
│       ├── specs/           # 设计文档
│       └── plans/           # 实现计划
├── crates/
│   ├── core/src/           # 7 files
│   ├── deepseek-client/src/ # 4 files
│   ├── tools/src/          # 7 files
│   ├── session-store/src/  # 4 files
│   ├── agent/src/          # 6 files
│   └── app/src/            # 3 files
└── tests/
```
