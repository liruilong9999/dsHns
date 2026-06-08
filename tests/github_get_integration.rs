//! `github_get` 工具集成测试。

use std::path::PathBuf;

use axum::extract::State;
use axum::routing::get;
use axum::Router;
use dshns::skill::manager::SkillManager;
use dshns::tools::builtin::GithubGetTool;
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
async fn should_get_github_api_with_mock_server() {
    #[derive(Clone)]
    struct AppState {
        token: String,
    }

    async fn github_me(
        State(state): State<AppState>,
        headers: axum::http::HeaderMap,
    ) -> String {
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if auth == format!("Bearer {}", state.token) {
            r#"{"login":"demo-user"}"#.to_string()
        } else {
            r#"{"error":"unauthorized"}"#.to_string()
        }
    }

    let token = "demo-token".to_string();
    let app = Router::new()
        .route("/user", get(github_me))
        .with_state(AppState {
            token: token.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定 GitHub 模拟端口失败");
    let addr = listener.local_addr().expect("读取 GitHub 模拟端口失败");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("启动 GitHub 模拟服务失败");
    });

    std::env::set_var("GITHUB_TOKEN", token);
    std::env::set_var("GITHUB_API_BASE_URL", format!("http://{}", addr));

    let context = build_context(&format!(
        "target/test_github_session_{}",
        uuid::Uuid::new_v4()
    ));
    let tool = GithubGetTool;
    let result = tool
        .handle(json!({ "endpoint": "/user" }), &context)
        .await
        .expect("github_get 执行失败");
    assert!(result.contains("demo-user"));

    std::env::remove_var("GITHUB_TOKEN");
    std::env::remove_var("GITHUB_API_BASE_URL");
    server.abort();
}
