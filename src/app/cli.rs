//! CLI 交互层实现。

use std::io::{self, Write};

use anyhow::Result;

use crate::app::harness::Harness;

/// CLI 应用。
pub struct CliApp {
    /// 应用主控器。
    harness: Harness,
}

impl CliApp {
    /// 创建 CLI 应用。
    pub fn new(harness: Harness) -> Self {
        Self { harness }
    }

    /// 启动 REPL 循环。
    pub async fn run(&mut self) -> Result<()> {
        println!("欢迎使用 dsHns Rust 版 Harness。");
        println!(
            "当前工作目录：{}",
            self.harness.settings().workspace_root.display()
        );
        println!("输入 /help 查看命令帮助。");

        loop {
            print!("dshns> ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if input.is_empty() {
                continue;
            }

            if input.starts_with('/') {
                if self.handle_command(input)? {
                    break;
                }
                continue;
            }

            let turn_result = {
                let run_future = self.harness.run_user_input(input);
                tokio::pin!(run_future);
                tokio::select! {
                    result = &mut run_future => Some(result),
                    _ = tokio::signal::ctrl_c() => None,
                }
            };

            match turn_result {
                Some(Ok(outcome)) => {
                    println!("{}", outcome.final_message);
                    println!(
                        "统计：输入 Token 约 {}，输出 Token {}，压缩触发：{}",
                        outcome.input_tokens,
                        outcome.output_tokens,
                        if outcome.compacted { "是" } else { "否" }
                    );
                }
                Some(Err(error)) => {
                    println!("执行失败：{}", error);
                }
                None => {
                    self.harness.cancel_current_turn()?;
                    println!("已取消当前轮执行。");
                }
            }
        }

        Ok(())
    }

    /// 处理 CLI 命令，返回值表示是否退出应用。
    fn handle_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        match parts.as_slice() {
            ["/help"] => self.print_help(),
            ["/quit"] => return Ok(true),
            ["/pwd"] => println!("{}", self.harness.settings().workspace_root.display()),
            ["/status"] => {
                if let Some(session) = self.harness.current_session() {
                    println!(
                        "当前会话：{}（{}），状态：{}，轮次：{}",
                        session.name,
                        session.id,
                        session.status.as_str(),
                        session.round
                    );
                    if let Ok(Some(stats)) = self.harness.latest_current_token_usage() {
                        println!(
                            "Token 统计：输入 {}，输出 {}，缓存命中率 {:.2}%，剩余上下文 {}",
                            stats.input_tokens,
                            stats.output_tokens,
                            stats.cache_hit_rate * 100.0,
                            stats.remaining_context
                        );
                    } else {
                        println!("Token 统计：暂无已执行轮次的统计数据");
                    }
                } else {
                    println!("当前尚未选择会话。");
                }
            }
            ["/create", name] => {
                let session = self.harness.create_session(name)?;
                println!("已创建会话：{}（{}）", session.name, session.id);
            }
            ["/session", "list"] | ["/sessions"] => {
                let sessions = self.harness.list_sessions()?;
                if sessions.is_empty() {
                    println!("当前没有可用会话。");
                } else {
                    for session in sessions {
                        println!(
                            "- {} | {} | 模型 {} | 状态 {} | 轮次 {}",
                            session.name,
                            session.id,
                            session.model,
                            session.status.as_str(),
                            session.round
                        );
                    }
                }
            }
            ["/session", "use", key] => {
                let session = self.harness.use_session(key)?;
                println!("已切换到会话：{}（{}）", session.name, session.id);
            }
            ["/session", "info"] => {
                if let Some(session) = self.harness.current_session() {
                    println!("会话名称：{}", session.name);
                    println!("会话标识：{}", session.id);
                    println!("模型：{}", session.model);
                    println!("审批模式：{}", session.approval_mode.as_str());
                    println!("工作目录：{}", session.working_directory);
                    println!("会话目录：{}", session.session_dir.display());
                } else {
                    println!("当前尚未选择会话。");
                }
            }
            ["/session", "delete", key] => {
                let audit_id = self.harness.delete_session(key)?;
                println!("会话已删除，审计记录：{}", audit_id);
            }
            ["/session", "restore", key] => {
                let session = self.harness.restore_session(key)?;
                println!("会话已恢复：{}（{}）", session.name, session.id);
            }
            ["/workspace", "list"] => {
                let workspaces = self.harness.list_workspaces()?;
                if workspaces.is_empty() {
                    println!("当前没有工作区记录。");
                } else {
                    for workspace in workspaces {
                        println!(
                            "- {} | {} | {}",
                            workspace.project_name, workspace.id, workspace.project_path
                        );
                    }
                }
            }
            ["/workspace", "delete", key] => {
                let audit_id = self.harness.delete_workspace(key)?;
                println!("工作区已删除，审计记录：{}", audit_id);
            }
            ["/workspace", "restore", key] => {
                let workspace = self.harness.restore_workspace(key)?;
                println!(
                    "工作区已恢复：{}（{}）",
                    workspace.project_name, workspace.id
                );
            }
            ["/audit", "list"] => {
                let audits = self.harness.list_deletion_audits(None)?;
                if audits.is_empty() {
                    println!("当前没有删除审计记录。");
                } else {
                    for audit in audits {
                        println!(
                            "- {} | {} | {} | 恢复时间：{}",
                            audit.id,
                            audit.target_type,
                            audit.target_id,
                            audit.restored_at.unwrap_or_else(|| "未恢复".to_string())
                        );
                    }
                }
            }
            ["/recovery", "log"] => {
                let log = self.harness.read_recovery_log()?;
                println!("{}", log);
            }
            ["/event", "list"] => {
                let events = self.harness.list_current_events()?;
                if events.is_empty() {
                    println!("当前会话没有事件记录。");
                } else {
                    for event in events {
                        println!(
                            "- {} | {:?} | 轮次 {} | {}",
                            event.event_id, event.event_type, event.round_no, event.timestamp
                        );
                    }
                }
            }
            ["/memory", "list"] => {
                let items = self.harness.list_current_working_memories()?;
                if items.is_empty() {
                    println!("当前会话没有工作记忆记录。");
                } else {
                    for item in items {
                        println!(
                            "- {} | version {} | before {} | after {} | {}",
                            item.id,
                            item.working_memory_version,
                            item.estimated_tokens_before,
                            item.estimated_tokens_after,
                            item.created_at
                        );
                    }
                }
            }
            ["/agent", "list"] => {
                let agents = self.harness.list_current_agents()?;
                if agents.is_empty() {
                    println!("当前会话没有子 Agent 记录。");
                } else {
                    for agent in agents {
                        println!(
                            "- {} | {:?} | level {} | {:?}",
                            agent.id, agent.mode, agent.level, agent.status
                        );
                    }
                }
            }
            ["/tool", "calls"] => {
                let calls = self.harness.list_current_tool_calls()?;
                if calls.is_empty() {
                    println!("当前会话没有工具调用记录。");
                } else {
                    for call in calls {
                        println!(
                            "- {} | {} | round {} | {} | success={}",
                            call.id, call.tool_name, call.round_no, call.status, call.success
                        );
                    }
                }
            }
            ["/tool", "results"] => {
                let results = self.harness.list_current_tool_results()?;
                if results.is_empty() {
                    println!("当前会话没有工具结果索引。");
                } else {
                    for result in results {
                        println!(
                            "- {} | {} | externalized={} | chars={} | bytes={}",
                            result.tool_call_id,
                            result.tool_name,
                            result.externalized,
                            result.char_count,
                            result.byte_count
                        );
                    }
                }
            }
            ["/tool", "body", tool_call_id] => {
                let content = self.harness.read_tool_result_body(tool_call_id)?;
                println!("{}", content);
            }
            ["/tool", "read", handle] => {
                let content = self.harness.read_tool_result_handle(handle)?;
                println!("{}", content);
            }
            ["/models"] => {
                for model in self.harness.models() {
                    println!("- {}", model);
                }
            }
            ["/model", "check", model] => match self.harness.check_model(model) {
                Ok(()) => println!("模型校验通过：{}", model),
                Err(error) => println!("模型校验失败：{}", error),
            },
            ["/mode"] => {
                println!("当前审批模式：{}", self.harness.current_mode().as_str());
            }
            ["/cancel"] => {
                self.harness.cancel_current_turn()?;
                println!("已标记当前会话为已取消。");
            }
            ["/clear"] => {
                print!("\x1B[2J\x1B[H");
                io::stdout().flush()?;
            }
            _ => {
                println!("未知命令：{}，请输入 /help 查看帮助。", input);
            }
        }

        Ok(false)
    }

    /// 打印帮助信息。
    fn print_help(&self) {
        println!("/create <name>            创建并选中新会话");
        println!("/session list             列出全部会话");
        println!("/session use <id|name>    切换会话");
        println!("/session info             查看当前会话详情");
        println!("/session delete <id|name> 删除会话并记录审计");
        println!("/session restore <audit>  从审计记录恢复会话");
        println!("/workspace list           查看工作区元数据");
        println!("/workspace delete <key>   删除工作区及会话元数据");
        println!("/workspace restore <key>  从审计记录恢复工作区");
        println!("/audit list               查看删除审计记录");
        println!("/recovery log             查看启动恢复日志");
        println!("/event list               查看当前会话事件");
        println!("/memory list              查看当前会话工作记忆");
        println!("/agent list               查看当前会话子 Agent");
        println!("/tool calls               查看当前会话工具调用记录");
        println!("/tool results             查看当前会话工具结果索引");
        println!("/tool body <tool_call_id> 按工具调用标识读取工具结果正文");
        println!("/tool read <handle>       读取 tool:call_xxx 工具结果句柄");
        println!("/status                   查看当前状态");
        println!("/pwd                      查看当前工作目录");
        println!("/models                   查看可用模型");
        println!("/model check <model>      校验模型是否允许");
        println!("/mode                     查看当前审批模式");
        println!("/cancel                   标记当前轮为已取消");
        println!("/clear                    清屏");
        println!("/quit                     退出 CLI");
    }
}
