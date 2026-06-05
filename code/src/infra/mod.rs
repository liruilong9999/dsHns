//! 基础设施层模块定义。
//!
//! 当前阶段优先实现配置加载能力。

/// 配置加载模块，负责系统默认值与环境变量读取。
pub mod config;

/// 数据库模块，负责 `SQLite` 连接管理与迁移执行。
pub mod db;

/// 子智能体管理模块，负责父子关系与生命周期流转。
pub mod agent_management;

/// 事件总线模块，负责按会话和智能体路由事件。
pub mod event_bus;

/// 仓储模块，负责目录、会话与消息的持久化访问。
pub mod repository;

/// 提示词装配模块，负责 AGENTS、技能元信息与上下文拼接。
pub mod prompting;

/// 工具系统模块，负责默认工具注册、参数校验与执行调度。
pub mod tool_system;
