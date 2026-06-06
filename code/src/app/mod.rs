//! 应用层模块定义。
//!
//! 当前版本保留既有公开模块路径，同时把实现按功能目录拆分到 `agent`、`cli`
//! 和 `session` 下，兼顾后续扩展性与现有调用兼容性。

/// 智能体运行相关应用模块。
pub mod agent;

/// CLI 交互相关应用模块。
pub mod cli;

/// 会话服务相关应用模块。
pub mod session;

/// 兼容旧路径的智能体执行模块导出。
pub mod agent_runner {
    pub use super::agent::runner::*;
}

/// 兼容旧路径的目录与会话服务模块导出。
pub mod workspace_session_service {
    pub use super::session::service::*;
}
