//! DeepSeek 专属 Agent 后端可执行入口。
//!
//! 当前阶段提供基础 CLI 主循环，后续再接入更完整的模型与工具链路。

use dshns_agent::app::cli::{CliApplication, CliResponse};
use dshns_agent::infra::config::AppConfig;
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use std::io::{self, BufRead, Write};

/// 程序入口函数。
fn main() {
    if let Err(error) = run_cli() {
        eprintln!("CLI 启动失败：{error}");
    }
}

/// 运行基础 CLI 主循环。
fn run_cli() -> Result<(), String> {
    let current_directory =
        std::env::current_dir().map_err(|error| format!("读取当前工作目录失败：{error}"))?;
    let database_path = current_directory.join(".dshns").join("runtime.sqlite3");

    let database = SqliteDatabase::open(DatabaseTarget::File(database_path))
        .map_err(|error| format!("打开运行数据库失败：{error}"))?;
    database
        .initialize()
        .map_err(|error| format!("初始化运行数据库失败：{error}"))?;

    let config = AppConfig::load();
    let mut cli = CliApplication::new(
        &database,
        &config,
        current_directory.to_string_lossy().to_string(),
    );

    println!("DeepSeek 专属 Agent CLI 启动中，输入 /quit 可退出。");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line_result in stdin.lock().lines() {
        let line = line_result.map_err(|error| format!("读取输入失败：{error}"))?;
        if line.trim().is_empty() {
            continue;
        }

        match cli.handle_input(&line) {
            Ok(response) => {
                writeln!(stdout, "{}", render_response(&response))
                    .map_err(|error| format!("输出响应失败：{error}"))?;
                stdout
                    .flush()
                    .map_err(|error| format!("刷新输出失败：{error}"))?;

                if matches!(response, CliResponse::Quit { quit: true }) {
                    break;
                }
            }
            Err(error) => {
                writeln!(stdout, "命令处理失败：{error}")
                    .map_err(|write_error| format!("输出错误信息失败：{write_error}"))?;
                stdout
                    .flush()
                    .map_err(|flush_error| format!("刷新错误输出失败：{flush_error}"))?;
            }
        }
    }

    Ok(())
}

/// 把 CLI 响应格式化为中文输出文本。
fn render_response(response: &CliResponse) -> String {
    match response {
        CliResponse::TextAccepted {
            session_id,
            round_id,
            created_new_session,
            display_state,
        } => format!(
            "{} 已接收输入，session_id={}，round_id={}，created_new_session={}",
            display_state.prefix_label(),
            session_id,
            round_id,
            created_new_session
        ),
        CliResponse::ModelsListed { models } => {
            format!("可用模型：{}", models.join("、"))
        }
        CliResponse::SessionsListed {
            workspace_id,
            sessions,
        } => {
            let session_titles = sessions
                .iter()
                .map(|session| format!("{}({})", session.title, session.session_id))
                .collect::<Vec<_>>();
            format!(
                "目录 {} 下的会话：{}",
                workspace_id,
                session_titles.join("、")
            )
        }
        CliResponse::ModelChanged {
            session_id,
            current_model,
            context_limit,
        } => format!(
            "会话 {} 已切换模型为 {}，上下文上限为 {}。",
            session_id, current_model, context_limit
        ),
        CliResponse::ModeChanged {
            session_id,
            session_approval_mode,
        } => format!(
            "会话 {} 已切换审批模式为 {}。",
            session_id, session_approval_mode
        ),
        CliResponse::Quit { .. } => "CLI 已退出。".to_string(),
    }
}
