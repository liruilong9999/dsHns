//! `MCP` 客户端成功链路集成测试。

use std::path::PathBuf;

use axum::routing::{get, post};
use axum::{Json, Router};
use dshns::mcp::client::{McpClientManager, McpServerConfig};
use dshns::utils::fs::write_utf8;
use serde_json::json;
use tokio::net::TcpListener;

#[tokio::test]
async fn should_connect_and_call_mcp_tool() {
    async fn capabilities() -> Json<serde_json::Value> {
        Json(json!({
            "tools": [
                {
                    "name": "echo",
                    "description": "echo tool"
                }
            ]
        }))
    }

    async fn echo(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
        Json(json!({
            "echo": payload
        }))
    }

    let app = Router::new()
        .route("/capabilities", get(capabilities))
        .route("/tools/echo", post(echo));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定 MCP 模拟端口失败");
    let addr = listener.local_addr().expect("读取 MCP 模拟端口失败");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("启动 MCP 模拟服务失败");
    });

    let workspace = PathBuf::from(format!(
        "target/test_mcp_workspace_{}",
        uuid::Uuid::new_v4()
    ));
    let session_dir = workspace.join("session");
    let servers_path = workspace.join(".dshns").join("mcp_servers.json");
    let config = vec![McpServerConfig {
        id: "demo".to_string(),
        name: "demo".to_string(),
        base_url: format!("http://{}", addr),
        enabled: true,
        capabilities_endpoint: None,
        tools_endpoint: None,
        api_key_env: None,
    }];
    write_utf8(
        &servers_path,
        &serde_json::to_string_pretty(&config).expect("序列化 MCP 配置失败"),
    )
    .expect("写入 MCP 配置失败");

    let manager = McpClientManager::new(workspace, session_dir);
    let state = manager
        .connect_server("demo")
        .await
        .expect("连接 MCP 服务端失败");
    assert_eq!(state.status, "connected");
    assert!(state.capabilities_json.contains("echo"));

    let result = manager
        .call_tool("demo", "echo", json!({"message":"hello"}))
        .await
        .expect("调用 MCP 工具失败");
    assert_eq!(
        result
            .get("echo")
            .and_then(|value| value.get("message"))
            .and_then(serde_json::Value::as_str),
        Some("hello")
    );

    server.abort();
}
