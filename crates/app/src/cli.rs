use std::path::PathBuf;
use clap::Parser;

#[derive(Parser)]
#[command(name = "dshns", about = "DeepSeek Agent", version = "0.1.0")]
pub struct Cli {
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,
    #[arg(short = 'c', long = "continue")]
    pub resume: bool,
    #[arg(short = 'm', long = "model")]
    pub model: Option<String>,
    #[arg(short = 'd', long = "dir")]
    pub working_dir: Option<PathBuf>,
    #[arg(long = "sessions")]
    pub list_sessions: bool,
    #[arg(long = "resume-session")]
    pub resume_session: Option<String>,
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}
