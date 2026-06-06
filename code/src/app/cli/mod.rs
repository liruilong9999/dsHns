//! 命令行交互模块。
//!
//! 该目录聚合 `CLI` 输入解析、命令路由、显示状态与后续渲染扩展能力。

/// CLI 应用主入口。
pub mod application;

pub use application::*;
