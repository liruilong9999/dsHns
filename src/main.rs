//! 可执行程序入口。
//!
//! 当前阶段以 CLI 为主要交付形态，因此主函数仅负责启动应用层。

/// 程序主入口，负责初始化异步运行时并启动 CLI。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dshns::app::run().await
}
