use std::sync::Arc;
use tokio::sync::mpsc;
use dshns_core::event::AgentEvent;
use dshns_core::message::Message;
use dshns_core::tool::ToolStatus;
use dshns_agent::agent_loop::AgentLoop;
use rustyline::{Editor, error::ReadlineError, history::DefaultHistory};

pub struct Repl {
    editor: Editor<(), DefaultHistory>,
    agent: Arc<AgentLoop>,
    /// 会话历史（不含 system prompt），每轮对话后累积
    history: Vec<Message>,
}

impl Repl {
    pub fn new(agent: Arc<AgentLoop>) -> Self {
        let mut editor = Editor::<(), DefaultHistory>::new().unwrap();
        let _ = editor.load_history(history_path().as_deref().unwrap_or(""));
        Self { editor, agent, history: Vec::new() }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("dsHns DeepSeek Agent -- /help 帮助, /exit 退出");
        loop {
            let line = match self.editor.readline("\n> ") {
                Ok(l) => l.trim().to_string(),
                Err(ReadlineError::Interrupted) => { println!("\n已中断"); continue; }
                Err(ReadlineError::Eof) => { println!("\n再见!"); break; }
                Err(e) => { eprintln!("错误: {}", e); break; }
            };
            if line.is_empty() { continue; }
            self.editor.add_history_entry(&line)?;

            if let Some(cmd) = line.strip_prefix('/') {
                match cmd {
                    "exit" | "e" | "quit" | "q" => break,
                    "help" | "h" => Self::help(),
                    "clear" => print!("\x1B[2J\x1B[1;1H"),
                    _ => self.process(&line).await?,
                }
            } else {
                self.process(&line).await?;
            }
        }
        let _ = self.editor.save_history(history_path().as_deref().unwrap_or(""));
        Ok(())
    }

    async fn process(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let agent = self.agent.clone();
        let owned = input.to_string();
        let history = std::mem::take(&mut self.history);
        let handle = tokio::spawn(async move { agent.run(&owned, history, tx).await });

        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Thinking(d) => { print!("{}", d); use std::io::Write; std::io::stdout().flush().ok(); }
                AgentEvent::ToolCallStart { name, .. } => println!("\n  🔧 {}", name),
                AgentEvent::ToolBlocked { reason, .. } => println!("\n  🚫 {}", reason),
                AgentEvent::ToolExecution { status, summary, .. } => match status {
                    ToolStatus::Success => println!("  ✓ {}", summary),
                    ToolStatus::Error { reason } => println!("  ✗ {}", reason),
                    _ => println!("  ℹ {:?}", status),
                },
                AgentEvent::TurnComplete { usage, tool_rounds } => println!("\n  [{} tokens | {} 轮工具]", usage.total_tokens, tool_rounds),
                AgentEvent::Error(m) => eprintln!("\n  ✗ {}", m),
                AgentEvent::SessionComplete => break,
                _ => {}
            }
        }
        println!();

        // 保存本轮对话历史，供下一轮使用
        match handle.await {
            Ok(Ok(outcome)) => {
                // 去掉开头的 system prompt，后续作为历史传入
                self.history = outcome.messages.into_iter()
                    .skip_while(|m| matches!(m, Message::System { .. }))
                    .collect();
            }
            Ok(Err(e)) => eprintln!("Agent 错误: {}", e),
            Err(e) => eprintln!("任务错误: {}", e),
        }
        Ok(())
    }

    fn help() {
        println!("/help, /h  帮助  /exit, /e  退出  /clear  清屏  Ctrl+C  中断  输入内容  发给 AI");
    }
}

fn history_path() -> Option<String> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    Some(format!("{}/.dsHns_rs/.history", home))
}
