//! 应用层模块定义。
//!
//! 当前阶段先保留模块边界，后续任务再逐步填充具体实现。

/// 智能体单轮执行器，负责提示词装配、模型调用与工具往返。
pub mod agent_runner;

/// CLI 应用层，负责命令与普通输入解析。
pub mod cli;

/// 目录与会话服务模块，负责目录和会话的基础应用编排。
pub mod workspace_session_service;
