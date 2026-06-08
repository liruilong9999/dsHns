//! 应用层模块。

/// HTTP 服务层。
pub mod api;
/// CLI 交互层。
pub mod cli;
/// 应用主控层。
pub mod harness;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use crate::app::api::ApiApp;
use crate::app::cli::CliApp;
use crate::app::harness::Harness;

/// 应用运行入口。
pub async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let workspace_root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let args: Vec<String> = env::args().skip(1).collect();
    match parse_mode(&args)? {
        AppMode::Cli => {
            let harness = Harness::new(workspace_root)?;
            let mut app = CliApp::new(harness);
            app.run().await
        }
        AppMode::Serve { addr } => {
            let app = ApiApp::new(workspace_root)?;
            app.run(addr).await
        }
    }
}

enum AppMode {
    Cli,
    Serve { addr: SocketAddr },
}

fn parse_mode(args: &[String]) -> Result<AppMode> {
    if args.first().map(String::as_str) != Some("serve") {
        return Ok(AppMode::Cli);
    }

    let host = args
        .iter()
        .position(|value| value == "--host")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = args
        .iter()
        .position(|value| value == "--port")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = format!("{}:{}", host, port).parse()?;
    Ok(AppMode::Serve { addr })
}
