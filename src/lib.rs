//! 应用库入口。
//!
//! 本文件负责统一暴露各层模块，便于 CLI、后续服务层与测试复用。

/// Agent 执行层模块。
pub mod agent;
/// 应用层模块。
pub mod app;
/// 审批层模块。
pub mod approval;
/// 配置层模块。
pub mod config;
/// 领域模型模块。
pub mod domain;
/// IPC 事件契约模块。
pub mod ipc;
/// 大模型客户端模块。
pub mod llm;
/// MCP 客户端模块。
pub mod mcp;
/// 持久化模块。
pub mod persistence;
/// 提示词组装模块。
pub mod prompt;
/// 会话管理模块。
pub mod session;
/// Skill 管理模块。
pub mod skill;
/// 工具系统模块。
pub mod tools;
/// 通用工具模块。
pub mod utils;
