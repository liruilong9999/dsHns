//! DeepSeek 专属 Agent 后端可执行入口。
//!
//! 当前阶段提供基础 CLI 主循环，后续再接入更完整的模型与工具链路。

use dshns_agent::app::agent_runner::{AgentRoundRequest, AgentRunner, AgentRunnerConfig};
use dshns_agent::app::cli::{CliApplication, CliDisplayState, CliResponse};
use dshns_agent::app::workspace_session_service::{
    ChangeSessionApprovalModeRequest, WorkspaceSessionService,
};
use dshns_agent::infra::config::AppConfig;
use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};
use dshns_agent::infra::deepseek_gateway::DeepSeekGateway;
use dshns_agent::infra::event_bus::EventBus;
use dshns_agent::infra::metrics::SessionMetricsRepository;
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
    let workspace_service = WorkspaceSessionService::new(&database, &config);
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

    print_startup_banner();

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut pending_approval: Option<PendingApproval> = None;
    let mut last_plain_input: Option<(String, String, String, String)> = None;
    for line_result in stdin.lock().lines() {
        render_prompt(&mut stdout, &cli)?;
        let line = line_result.map_err(|error| format!("读取输入失败：{error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let is_command = line.trim().starts_with('/');
        if matches!(line.trim(), "执行" | "确认执行" | "confirm" | "approve") {
            let approval = pending_approval.clone().or_else(|| {
                last_plain_input.clone().map(
                    |(session_id, original_input, previous_mode, round_id)| PendingApproval {
                        session_id,
                        original_input,
                        previous_mode,
                        round_id,
                    },
                )
            });
            if let Some(pending) = approval {
                workspace_service
                    .change_session_approval_mode(ChangeSessionApprovalModeRequest {
                        session_id: pending.session_id.clone(),
                        session_approval_mode: "auto".to_string(),
                    })
                    .map_err(|error| format!("切换审批模式失败：{error}"))?;
                if let Some(gateway) = &gateway {
                    let runner = AgentRunner::new(
                        &database,
                        &config,
                        gateway.clone(),
                        runner_config.clone(),
                    )
                    .with_event_bus(event_bus.clone());
                    let outcome = runner
                        .run_round(AgentRoundRequest {
                            session_id: pending.session_id.clone(),
                            agent_id: "AGT-0001".to_string(),
                            user_input: pending.original_input.clone(),
                            input_already_persisted: true,
                            existing_round_id: Some(pending.round_id.clone()),
                            approval_mode_override: Some(
                                dshns_agent::domain::tool::SessionApprovalMode::Auto,
                            ),
                        })
                        .map_err(|error| format!("审批后执行失败：{error}"))?;
                    workspace_service
                        .change_session_approval_mode(ChangeSessionApprovalModeRequest {
                            session_id: pending.session_id.clone(),
                            session_approval_mode: pending.previous_mode.clone(),
                        })
                        .map_err(|error| format!("恢复审批模式失败：{error}"))?;
                    render_round_outcome(
                        &database,
                        &event_bus,
                        &pending.session_id,
                        outcome,
                        &mut stdout,
                    )?;
                    pending_approval = None;
                    continue;
                }
            }
        }

        match cli.handle_input(&line) {
            Ok(response) => {
                if is_command {
                    writeln!(
                        stdout,
                        "{}",
                        colorize(&render_response(&response), CliDisplayState::Answer)
                    )
                    .map_err(|error| format!("输出响应失败：{error}"))?;
                    print_status_line(&database, &cli, &mut stdout)?;
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
                    let previous_mode = workspace_service
                        .get_session(&session_id)
                        .map_err(|error| format!("读取会话审批模式失败：{error}"))?
                        .session_approval_mode;
                    last_plain_input = Some((
                        session_id.clone(),
                        line.clone(),
                        previous_mode,
                        round_id.clone(),
                    ));
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
                                    existing_round_id: Some(round_id.clone()),
                                    approval_mode_override: None,
                                })
                                .map_err(|error| format!("智能体执行失败：{error}"))?;
                            if outcome.tool_responses.iter().any(|response| {
                                response.error_code.as_deref() == Some("APPROVAL_REQUIRED")
                            }) {
                                let previous_mode = workspace_service
                                    .get_session(&session_id)
                                    .map_err(|error| format!("读取会话审批模式失败：{error}"))?
                                    .session_approval_mode;
                                pending_approval = Some(PendingApproval {
                                    session_id: session_id.clone(),
                                    original_input: line.clone(),
                                    previous_mode,
                                    round_id: round_id.clone(),
                                });
                                writeln!(
                                    stdout,
                                    "{}",
                                    colorize(
                                        "本轮工具调用需要确认，输入“执行”或“确认执行”继续。",
                                        CliDisplayState::ToolFailure
                                    )
                                )
                                .map_err(|error| format!("输出审批提示失败：{error}"))?;
                            }
                            render_round_outcome(
                                &database,
                                &event_bus,
                                &session_id,
                                outcome,
                                &mut stdout,
                            )?;
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
fn render_event(event_type: &str, payload: &serde_json::Value) -> (CliDisplayState, String) {
    match event_type {
        "model_thinking_started" => (
            CliDisplayState::Thinking,
            if let Some(reasoning) = payload["reasoning_content"].as_str() {
                format!("{} {}", CliDisplayState::Thinking.prefix_label(), reasoning)
            } else {
                format!(
                    "{} 模型正在思考。",
                    CliDisplayState::Thinking.prefix_label()
                )
            },
        ),
        "tool_started" => (
            CliDisplayState::ToolRunning,
            format!(
                "{} 工具开始执行：{}",
                CliDisplayState::ToolRunning.prefix_label(),
                payload["tool_name"].as_str().unwrap_or("unknown")
            ),
        ),
        "tool_finished" => {
            let error_code = payload["error_code"].as_str().unwrap_or_default();
            let message = payload["message"].as_str().unwrap_or_default();
            let is_success = payload["status"]
                .as_str()
                .unwrap_or_default()
                .contains("Success");
            if error_code == "APPROVAL_REQUIRED" {
                return (
                    CliDisplayState::ToolFailure,
                    format!(
                        "{} 工具等待确认：{}",
                        CliDisplayState::ToolFailure.prefix_label(),
                        payload["tool_name"].as_str().unwrap_or("unknown")
                    ),
                );
            }
            let state = if is_success {
                CliDisplayState::ToolSuccess
            } else {
                CliDisplayState::ToolFailure
            };
            (
                state,
                format!(
                    "{} 工具执行{}：{}{}",
                    state.prefix_label(),
                    if is_success { "成功" } else { "失败" },
                    payload["tool_name"].as_str().unwrap_or("unknown"),
                    if !is_success && !message.is_empty() {
                        format!("，原因：{} ({})", message, error_code)
                    } else {
                        String::new()
                    }
                ),
            )
        }
        "assistant_output_delta" => (
            CliDisplayState::Answer,
            format!("{} 正在生成回答。", CliDisplayState::Answer.prefix_label()),
        ),
        "assistant_output_completed" => (
            CliDisplayState::Answer,
            format!("{} 回答生成完成。", CliDisplayState::Answer.prefix_label()),
        ),
        "metrics_updated" => (
            CliDisplayState::Answer,
            "[指标] 会话指标已刷新。".to_string(),
        ),
        _ => (CliDisplayState::Answer, format!("[事件] {}", event_type)),
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

#[derive(Clone)]
struct PendingApproval {
    session_id: String,
    original_input: String,
    previous_mode: String,
    round_id: String,
}

fn print_startup_banner() {
    println!("DeepSeek 专属 Agent CLI 启动中，输入 /quit 可退出。");
    println!("可用命令：/models  /model check <模型名>  /mode  /sessions  /quit");
    println!(
        "操作建议：先输入普通文本创建会话；若处于 ask 模式，工具审批后可输入“执行”或“确认执行”继续。"
    );
}

fn render_prompt(stdout: &mut io::Stdout, cli: &CliApplication<'_>) -> Result<(), String> {
    let session = cli.current_session_id().unwrap_or("NEW");
    let mode = cli
        .current_approval_mode()
        .map_err(|error| format!("读取当前审批模式失败：{error}"))?
        .unwrap_or_else(|| "ask".to_string());
    write!(stdout, "[{}|mode={}] > ", session, mode)
        .map_err(|error| format!("输出提示符失败：{error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("刷新提示符失败：{error}"))?;
    Ok(())
}

fn colorize(text: &str, state: CliDisplayState) -> String {
    let code = match state {
        CliDisplayState::Thinking => "33",
        CliDisplayState::ToolRunning => "37",
        CliDisplayState::ToolSuccess => "32",
        CliDisplayState::ToolFailure => "31",
        CliDisplayState::Answer => "37",
    };
    format!("\u{1b}[{}m{}\u{1b}[0m", code, text)
}

fn print_status_line(
    database: &SqliteDatabase,
    cli: &CliApplication<'_>,
    stdout: &mut io::Stdout,
) -> Result<(), String> {
    let Some(session_id) = cli.current_session_id() else {
        return Ok(());
    };
    print_status_line_for_session(database, session_id, stdout)
}

fn print_status_line_for_session(
    database: &SqliteDatabase,
    session_id: &str,
    stdout: &mut io::Stdout,
) -> Result<(), String> {
    let repository = SessionMetricsRepository::new(database.connection());
    let metric = repository
        .latest_by_session(session_id)
        .map_err(|error| format!("读取会话指标失败：{error}"))?;
    let line = if let Some(metric) = metric {
        format!(
            "状态栏 | 输入Token={} 输出Token={} 缓存命中率={:.2} 剩余上下文={}",
            metric.input_tokens,
            metric.output_tokens,
            metric.cache_hit_rate,
            metric.remaining_context
        )
    } else {
        "状态栏 | 输入Token=0 输出Token=0 缓存命中率=0.00 剩余上下文=0".to_string()
    };
    writeln!(stdout, "{}", colorize(&line, CliDisplayState::Answer))
        .map_err(|error| format!("输出状态栏失败：{error}"))?;
    Ok(())
}

fn render_round_outcome(
    database: &SqliteDatabase,
    event_bus: &EventBus<'_>,
    session_id: &str,
    outcome: dshns_agent::app::agent_runner::AgentRoundOutcome,
    stdout: &mut io::Stdout,
) -> Result<(), String> {
    for event in event_bus
        .drain_session_events(session_id)
        .map_err(|error| format!("读取事件失败：{error}"))?
    {
        let (state, text) = render_event(&event.event_type, &event.payload);
        if state == CliDisplayState::Thinking && event.payload["reasoning_content"].is_string() {
            let prefix = colorize(
                CliDisplayState::Thinking.prefix_label(),
                CliDisplayState::Thinking,
            );
            write!(stdout, "{} ", prefix).map_err(|error| format!("输出思考前缀失败：{error}"))?;
            for ch in event.payload["reasoning_content"]
                .as_str()
                .unwrap_or_default()
                .chars()
            {
                write!(
                    stdout,
                    "{}",
                    colorize(&ch.to_string(), CliDisplayState::Thinking)
                )
                .map_err(|error| format!("输出思考内容失败：{error}"))?;
            }
            writeln!(stdout).map_err(|error| format!("输出思考换行失败：{error}"))?;
        } else {
            writeln!(stdout, "{}", colorize(&text, state))
                .map_err(|error| format!("输出事件失败：{error}"))?;
        }
    }
    if let Some(final_text) = outcome.final_text {
        let prefix = colorize(
            CliDisplayState::Answer.prefix_label(),
            CliDisplayState::Answer,
        );
        write!(stdout, "{} ", prefix).map_err(|error| format!("输出回答前缀失败：{error}"))?;
        for ch in final_text.chars() {
            write!(stdout, "{}", ch).map_err(|error| format!("流式输出回答失败：{error}"))?;
        }
        writeln!(stdout).map_err(|error| format!("输出换行失败：{error}"))?;
    }
    print_status_line_for_session(database, session_id, stdout)?;
    stdout
        .flush()
        .map_err(|error| format!("刷新输出失败：{error}"))?;
    Ok(())
}
