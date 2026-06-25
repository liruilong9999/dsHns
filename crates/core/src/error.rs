use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DshnsError {
    #[error("未设置 DEEPSEEK_API_KEY 环境变量")]
    NoApiKey,
    #[error("配置错误: {0}")]
    Config(String),
    #[error("API 认证失败 (401): {0}")]
    ApiAuth(String),
    #[error("API 速率限制 (429): {0}s 后重试")]
    ApiRateLimited(u64),
    #[error("API 服务器错误: {0}")]
    ApiServer(String),
    #[error("网络错误: {0}")]
    Network(String),
    #[error("SSE 解析错误: {0}")]
    SseParse(String),
    #[error("工具执行错误: {0}")]
    Tool(String),
    #[error("会话未找到: {0}")]
    SessionNotFound(String),
    #[error("会话损坏: {0}")]
    SessionCorrupted(PathBuf),
    #[error("工具循环卡住: 连续失败 {failures} 次")]
    ToolLoopStuck { failures: u32 },
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}
