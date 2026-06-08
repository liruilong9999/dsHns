//! API 扩展能力集成测试。

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dshns::app::api::ApiApp;
use dshns::utils::fs::{ensure_directory, write_utf8};
use tower::ServiceExt;

#[tokio::test]
async fn should_query_tool_result_body_and_delete_restore_workspace() {
    let workspace = PathBuf::from(format!(
        "target/test_api_extended_workspace_{}",
        uuid::Uuid::new_v4()
    ));
    let app = ApiApp::new(workspace.clone()).expect("创建 API 应用失败");
    let router = app.router();

    let create = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"api-ext-demo"}"#))
                .unwrap(),
        )
        .await
        .expect("创建会话请求失败");
    assert_eq!(create.status(), StatusCode::OK);
    let create_body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .expect("读取创建响应失败");
    let created: serde_json::Value =
        serde_json::from_slice(&create_body).expect("解析创建响应失败");
    let session_id = created
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("缺少 session id")
        .to_string();
    let session_dir = created
        .get("session_dir")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .expect("缺少 session_dir");

    let tool_results_dir = session_dir.join("tool_results");
    ensure_directory(&tool_results_dir).expect("创建 tool_results 目录失败");
    let body_path = tool_results_dir.join("call_success.txt");
    write_utf8(&body_path, "external tool body").expect("写入工具结果正文失败");
    let index_content = serde_json::to_string_pretty(&vec![serde_json::json!({
        "tool_call_id": "call_success",
        "tool_name": "read_file",
        "handle": "tool:call_success",
        "body_file_path": body_path.to_string_lossy(),
        "projection_type": "Summary",
        "projection_content": "summary",
        "summary": "ok",
        "preview_head": "summary",
        "preview_tail": "summary",
        "char_count": 18,
        "byte_count": 18,
        "success": true,
        "truncated": true,
        "externalized": true,
        "updated_at": "2026-01-01T00:00:00Z"
    })])
    .expect("序列化索引失败");
    write_utf8(&tool_results_dir.join("index.json"), &index_content).expect("写入工具结果索引失败");

    let tool_result_body = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/sessions/{}/tool-results/{}",
                    session_id, "call_success"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("请求工具结果正文失败");
    assert_eq!(tool_result_body.status(), StatusCode::OK);
    let tool_result_body_bytes = axum::body::to_bytes(tool_result_body.into_body(), usize::MAX)
        .await
        .expect("读取工具结果正文响应失败");
    let tool_result_json: serde_json::Value =
        serde_json::from_slice(&tool_result_body_bytes).expect("解析工具结果正文响应失败");
    assert_eq!(
        tool_result_json
            .get("output")
            .and_then(serde_json::Value::as_str),
        Some("external tool body")
    );

    let tool_calls = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/sessions/{}/tool-calls", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("请求 tool-calls 失败");
    assert_eq!(tool_calls.status(), StatusCode::OK);

    let workspaces = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/workspaces")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("请求 workspaces 失败");
    assert_eq!(workspaces.status(), StatusCode::OK);
    let workspaces_body = axum::body::to_bytes(workspaces.into_body(), usize::MAX)
        .await
        .expect("读取 workspaces 响应失败");
    let workspaces_json: serde_json::Value =
        serde_json::from_slice(&workspaces_body).expect("解析 workspaces 响应失败");
    let workspace_id = workspaces_json
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("id"))
        .and_then(serde_json::Value::as_str)
        .expect("缺少 workspace id")
        .to_string();

    let delete_workspace = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/workspaces/{}", workspace_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("删除 workspace 请求失败");
    assert_eq!(delete_workspace.status(), StatusCode::OK);
    let delete_workspace_body =
        axum::body::to_bytes(delete_workspace.into_body(), usize::MAX)
            .await
            .expect("读取删除 workspace 响应失败");
    let deleted_workspace: serde_json::Value = serde_json::from_slice(&delete_workspace_body)
        .expect("解析删除 workspace 响应失败");
    let workspace_audit_id = deleted_workspace
        .get("audit_id")
        .and_then(serde_json::Value::as_str)
        .expect("缺少 workspace audit_id")
        .to_string();

    let restore_workspace = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/workspaces/{}/restore", workspace_audit_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("恢复 workspace 请求失败");
    assert_eq!(restore_workspace.status(), StatusCode::OK);

    let recovery_log = router
        .oneshot(
            Request::builder()
                .uri("/recovery-log")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("请求 recovery-log 失败");
    assert_eq!(recovery_log.status(), StatusCode::OK);
}
