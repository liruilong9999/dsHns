//! DeepSeek 专属 Agent 后端可执行入口。
//!
//! 当前阶段提供基础 CLI 主循环，后续再接入更完整的模型与工具链路。

use dshns_agent::app::agent_runner::{AgentRoundRequest, AgentRunner, AgentRunnerConfig};
use dshns_agent::app::cli::{CliApplication, CliResponse, CliDisplayState};
use dshns_agent::infra::config::AppConfig;
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::deepseek_gateway::DeepSeekGateway;
use dshns_agent::infra::event_bus::EventBus;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

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
    let event_bus = EventBus::new(&database);
    let gateway = config
        .model_gateway()
        .api_key()
        .cloned()
        .map(DeepSeekGateway::new)
        .transpose()
        .map_err(|error| format!("创建 DeepSeek 网关失败：{error}"))?;
    let runner_config = AgentRunnerConfig {
        workspace_root_path: current_directory.clone(),
        skill_root_path: resolve_skill_root(&current_directory),
        global_agents_path: None,
        system_prompt: "你是 DeepSeek 专属 Agent，请在当前工作区中对用户提供开发帮助。".to_string(),
    };

    println!("DeepSeek 专属 Agent CLI 启动中，输入 /quit 可退出。");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line_result in stdin.lock().lines() {
        let line = line_result.map_err(|error| format!("读取输入失败：{error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let is_command = line.trim().starts_with('/');

        match cli.handle_input(&line) {
            Ok(response) => {
                if is_command {
                    writeln!(stdout, "{}", render_response(&response))
                        .map_err(|error| format!("输出响应失败：{error}"))?;
                    stdout
                        .flush()
                        .map_err(|error| format!("刷新输出失败：{error}"))?;

                    if matches!(response, CliResponse::Quit { quit: true }) {
                        break;
                    }
                } else if let CliResponse::TextAccepted {
                    session_id,
                    round_id,
                    ..
                } = response
                {
                    event_bus
                        .register_session(&session_id)
                        .map_err(|error| format!("注册会话事件队列失败：{error}"))?;
                    match &gateway {
                        Some(gateway) => {
                            let runner = AgentRunner::new(
                                &database,
                                &config,
                                gateway.clone(),
                                runner_config.clone(),
                            )
                            .with_event_bus(event_bus.clone());
                            let outcome = runner
                                .run_round(AgentRoundRequest {
                                    session_id: session_id.clone(),
                                    agent_id: "AGT-0001".to_string(),
                                    user_input: line.clone(),
                                    input_already_persisted: true,
                                    existing_round_id: Some(round_id),
                                })
                                .map_err(|error| format!("智能体执行失败：{error}"))?;
                            for event in event_bus
                                .drain_session_events(&session_id)
                                .map_err(|error| format!("读取事件失败：{error}"))?
                            {
                                writeln!(stdout, "{}", render_event(&event.event_type))
                                    .map_err(|error| format!("输出事件失败：{error}"))?;
                            }
                            if let Some(final_text) = outcome.final_text {
                                writeln!(
                                    stdout,
                                    "{} {}",
                                    CliDisplayState::Answer.prefix_label(),
                                    final_text
                                )
                                .map_err(|error| format!("输出最终结果失败：{error}"))?;
                            }
                            stdout
                                .flush()
                                .map_err(|error| format!("刷新输出失败：{error}"))?;
                        }
                        None => {
                            writeln!(
                                stdout,
                                "模型网关不可用：{}",
                                config.model_gateway().user_facing_message()
                            )
                            .map_err(|error| format!("输出模型错误失败：{error}"))?;
                            stdout
                                .flush()
                                .map_err(|error| format!("刷新输出失败：{error}"))?;
                        }
                    }
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

/// 把事件类型转换为命令行可见文本。
fn render_event(event_type: &str) -> String {
    match event_type {
        "model_thinking_started" => format!("{} 模型正在思考。", CliDisplayState::Thinking.prefix_label()),
        "tool_started" => format!("{} 工具开始执行。", CliDisplayState::ToolRunning.prefix_label()),
        "tool_finished" => format!("{} 工具执行完成。", CliDisplayState::ToolSuccess.prefix_label()),
        "assistant_output_delta" => format!("{} 正在生成回答。", CliDisplayState::Answer.prefix_label()),
        "assistant_output_completed" => format!("{} 回答生成完成。", CliDisplayState::Answer.prefix_label()),
        "metrics_updated" => "[指标] 会话指标已刷新。".to_string(),
        _ => format!("[事件] {}", event_type),
    }
}

/// 解析默认技能根目录。
fn resolve_skill_root(workspace_root: &PathBuf) -> PathBuf {
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let codex_skill_root = PathBuf::from(user_profile).join(".codex").join("skills");
        if codex_skill_root.exists() {
            return codex_skill_root;
        }
    }

    workspace_root.join(".skills")
}
