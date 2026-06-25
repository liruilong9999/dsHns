mod cli;
mod repl;

use std::sync::Arc;
use clap::Parser;
use dshns_core::config::{AppConfig, ApprovalMode};
use dshns_core::error::DshnsError;
use dshns_deepseek_client::client::DeepSeekClient;
use dshns_tools::registry::ToolRegistry;
use dshns_tools::executor::ToolExecutor;
use dshns_tools::builtin::{
    read_file::ReadFileTool, write_file::WriteFileTool,
    exec_shell::ExecShellTool, search_code::SearchCodeTool,
};
use dshns_session_store::store::SessionStore;
use dshns_session_store::prompt::PromptLoader;
use dshns_agent::agent_loop::AgentLoop;
use dshns_agent::subagent::{AgentOpenTool, AgentCloseTool, AgentResultTool};

fn load_config() -> Result<AppConfig, DshnsError> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .map_err(|_| DshnsError::Config("HOME not found".into()))?;
    let dir = home.join(".dsHns_rs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.toml");
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .map_err(|e| DshnsError::Config(e.to_string()))?;
        toml::from_str(&s).map_err(|e| DshnsError::Config(e.to_string()))
    } else {
        let cfg = AppConfig::default();
        std::fs::write(&path, toml::to_string_pretty(&cfg).unwrap())?;
        eprintln!("已创建默认配置: {}", path.display());
        Ok(cfg)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = cli::Cli::parse();
    let mut config = load_config()?;
    if let Some(ref m) = cli.model {
        config.api.model = m.clone();
    }

    let api_key =
        std::env::var("DEEPSEEK_API_KEY").map_err(|_| "请设置 DEEPSEEK_API_KEY 环境变量")?;
    let working_dir = cli.working_dir.unwrap_or_else(|| std::env::current_dir().unwrap());
    let system_prompt = PromptLoader::load(&working_dir)?;

    let client = Arc::new(DeepSeekClient::new(api_key, config.api.clone())?);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(ExecShellTool::new(config.agent.tool_timeout_secs)));
    registry.register(Arc::new(SearchCodeTool));
    registry.register(Arc::new(AgentOpenTool));
    registry.register(Arc::new(AgentCloseTool));
    registry.register(Arc::new(AgentResultTool));

    let registry = Arc::new(registry);
    let executor = Arc::new(ToolExecutor::new(registry.clone(), config.agent.tool_timeout_secs));
    let mode = ApprovalMode::from_str(&config.mode.default);

    let agent = Arc::new(AgentLoop::new(
        client.clone(),
        executor.clone(),
        registry.clone(),
        config.agent.clone(),
        config.context.clone(),
        mode,
        1,
        system_prompt,
    ));

    if cli.list_sessions {
        let store = SessionStore::new()?;
        for s in store.list()? {
            println!(
                "{}  {}  ({}msgs)  {}",
                s.id.to_string().chars().take(8).collect::<String>(),
                s.title,
                s.message_count,
                s.updated_at.format("%Y-%m-%d %H:%M")
            );
        }
        return Ok(());
    }

    if let Some(prompt) = cli.prompt {
        println!("处理中...");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let a = agent.clone();
        tokio::spawn(async move { a.run(&prompt, vec![], tx).await });
        while let Some(ev) = rx.recv().await {
            match ev {
                dshns_core::event::AgentEvent::Thinking(d) => print!("{}", d),
                dshns_core::event::AgentEvent::SessionComplete => break,
                _ => {}
            }
        }
        println!();
    } else {
        repl::Repl::new(agent).run().await?;
    }

    Ok(())
}
