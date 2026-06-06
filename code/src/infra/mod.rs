//! 基础设施层模块定义。
//!
//! 当前版本将基础设施能力按配置、存储、提示装配、技能发现、工具系统等职责拆分，
//! 便于后续围绕单一能力持续扩展。

/// 配置加载模块，负责系统默认值与环境变量读取。
pub mod config;

/// 数据库模块，负责 `SQLite` 连接管理与迁移执行。
pub mod db;

/// 子智能体管理模块，负责父子关系与生命周期流转。
pub mod agent_management;

/// DeepSeek 实时模型网关。
pub mod deepseek_gateway;

/// 上下文管理模块，负责压缩触发与长结果预算。
pub mod context_management;

/// 事件总线模块，负责按会话和智能体路由事件。
pub mod event_bus;

/// 指标模块，负责会话指标快照与刷新。
pub mod metrics;

/// 持久化仓储模块，负责目录、会话与消息的数据访问。
pub mod repository;

/// 技能发现模块，负责统一技能扫描与元信息解析。
pub mod skills;

/// 提示词装配模块，负责 AGENTS、技能摘要与上下文拼接。
pub mod prompting;

/// 工具系统模块，负责默认工具注册、参数校验与执行调度。
pub mod tool_system;
