//! finance 工具集成测试。
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::FinanceTool;
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use dshns::utils::fs::ensure_directory;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn should_query_finance_with_mock_server() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("启动本地行情服务失败");
    let address = listener.local_addr().expect("读取本地地址失败");
    let server = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer);
            let body = r#"{
                "chart": {
                    "result": [{
                        "meta": {
                            "symbol": "AAPL",
                            "currency": "USD",
                            "regularMarketPrice": 210.5,
                            "previousClose": 208.3
                        },
                        "timestamp": [1735689600]
                    }]
                }
            }"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("写入本地行情响应失败");
        }
    });

    let workspace_root = PathBuf::from(format!("target/test_finance_workspace_{}", Uuid::new_v4()));
    let session_dir = workspace_root.join("session");
    ensure_directory(&session_dir).expect("创建会话目录失败");
    let context = ToolExecutionContext {
        workspace_root,
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    };

    std::env::set_var("YAHOO_FINANCE_BASE_URL", format!("http://{}", address));
    let tool = FinanceTool;
    let result = tool
        .handle(json!({ "symbol": "AAPL" }), &context)
        .await
        .expect("finance 执行失败");
    std::env::remove_var("YAHOO_FINANCE_BASE_URL");
    server.join().expect("本地行情服务线程失败");

    assert!(result.contains("\"symbol\": \"AAPL\""));
    assert!(result.contains("\"regular_market_price\": 210.5"));
}
