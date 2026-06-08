//! `fetch_url` 与 `web_run` 端到端集成测试。

use std::path::PathBuf;

use axum::routing::get;
use axum::Router;
use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::{FetchUrlTool, WebRunTool};
use dshns::tools::registry::{ToolExecutionContext, ToolHandler};
use serde_json::json;
use tokio::net::TcpListener;

fn build_context(session_dir: &str) -> ToolExecutionContext {
    let workspace_root = PathBuf::from("target/test_workspace");
    let session_dir = PathBuf::from(session_dir);
    std::fs::create_dir_all(&workspace_root).expect("创建测试工作区失败");
    std::fs::create_dir_all(&session_dir).expect("创建测试会话目录失败");

    ToolExecutionContext {
        workspace_root,
        session_dir,
        shell_program: "powershell".to_string(),
        skill_manager: SkillManager::new(Vec::new()),
    }
}

#[tokio::test]
async fn should_fetch_url_and_run_web_steps() {
    async fn index() -> &'static str {
        r#"
        <html>
          <body>
            <a href="/next">Go next</a>
            <div class="content">Hello Web</div>
          </body>
        </html>
        "#
    }

    async fn next() -> &'static str {
        r#"
        <html>
          <body>
            <div id="result">Done</div>
          </body>
        </html>
        "#
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/next", get(next));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定本地测试端口失败");
    let addr = listener.local_addr().expect("读取本地端口失败");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("启动测试服务失败");
    });

    let context = build_context(&format!(
        "target/test_web_run_session_{}",
        uuid::Uuid::new_v4()
    ));
    let base_url = format!("http://{}", addr);

    let fetch_tool = FetchUrlTool;
    let fetched = fetch_tool
        .handle(json!({ "url": format!("{}/", base_url) }), &context)
        .await
        .expect("fetch_url 执行失败");
    assert!(fetched.contains("Hello Web"));

    let web_run_tool = WebRunTool;
    let result = web_run_tool
        .handle(
            json!({
                "steps": [
                    { "action": "open", "url": format!("{}/", base_url) },
                    { "action": "find", "pattern": "Hello Web" },
                    { "action": "click", "text_contains": "Go next" },
                    { "action": "extract_text", "selector": "#result" }
                ]
            }),
            &context,
        )
        .await
        .expect("web_run 执行失败");
    assert!(result.contains("\"success\": true"));
    assert!(result.contains("Done"));

    server.abort();
}
