//! HTTP 服务层实现。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::config::settings::Settings;
use crate::domain::{
    AgentInstance, DeletionAudit, Session, SessionStatusSnapshot, ToolCallRecord, ToolResultRecord,
    WorkingMemoryEntry, WorkspaceDirectory,
};
use crate::ipc::bus::EventBus;
use crate::ipc::events::IpcEvent;
use crate::persistence::sqlite::SqliteStore;
use crate::prompt::assembler::PromptAssembler;
use crate::session::manager::SessionManager;
use crate::skill::manager::SkillManager;
use crate::utils::fs::read_optional_utf8;

/// 服务应用。
pub struct ApiApp {
    /// 服务状态。
    state: ApiState,
}

#[derive(Clone)]
struct ApiState {
    settings: Settings,
    session_manager: Arc<SessionManager>,
    prompt_assembler: PromptAssembler,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    workspace_root: String,
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    name: String,
}

#[derive(Serialize)]
struct RecoveryLogResponse {
    content: String,
}

#[derive(Serialize)]
struct ToolResultBodyResponse {
    tool_call_id: String,
    output: String,
}

#[derive(Serialize)]
struct DeleteResponse {
    audit_id: String,
}

impl ApiApp {
    /// 创建服务应用。
    pub fn new(workspace_root: PathBuf) -> Result<Self> {
        let settings = Settings::load(&workspace_root)?;
        let skill_manager = SkillManager::new(settings.skill_roots.clone());
        let prompt_assembler = PromptAssembler::new(workspace_root, skill_manager);
        let store = Arc::new(SqliteStore::new(&settings.database_path)?);
        let session_manager = Arc::new(SessionManager::new(settings.clone(), store));
        session_manager.repair_from_snapshots()?;
        Ok(Self {
            state: ApiState {
                settings,
                session_manager,
                prompt_assembler,
            },
        })
    }

    /// 启动 HTTP 服务。
    pub async fn run(self, addr: SocketAddr) -> Result<()> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, self.router()).await?;
        Ok(())
    }

    /// 构建路由，便于测试复用。
    pub fn router(&self) -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/sessions", get(list_sessions).post(create_session))
            .route(
                "/sessions/{session_id}",
                get(get_session).delete(delete_session),
            )
            .route("/sessions/{session_id}/status", get(get_session_status))
            .route("/sessions/{key}/restore", get(restore_session))
            .route("/sessions/{session_id}/events", get(list_events))
            .route("/sessions/{session_id}/memory", get(list_working_memories))
            .route("/sessions/{session_id}/agents", get(list_agents))
            .route("/sessions/{session_id}/tool-calls", get(list_tool_calls))
            .route(
                "/sessions/{session_id}/tool-results",
                get(list_tool_results),
            )
            .route(
                "/sessions/{session_id}/tool-results/{tool_call_id}",
                get(get_tool_result_body),
            )
            .route("/workspaces", get(list_workspaces))
            .route(
                "/workspaces/{key}",
                get(get_workspace).delete(delete_workspace),
            )
            .route("/workspaces/{key}/restore", get(restore_workspace))
            .route("/audits", get(list_audits))
            .route("/recovery-log", get(get_recovery_log))
            .with_state(self.state.clone())
    }
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        workspace_root: state.settings.workspace_root.to_string_lossy().to_string(),
    })
}

async fn list_sessions(State(state): State<ApiState>) -> Result<Json<Vec<Session>>, ApiError> {
    Ok(Json(state.session_manager.list_sessions()?))
}

async fn create_session(
    State(state): State<ApiState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<Session>, ApiError> {
    let prompt = state.prompt_assembler.assemble()?;
    let session = state.session_manager.create_session(
        &request.name,
        &state.settings.default_model,
        state.settings.default_approval_mode,
        state.settings.default_stream_output,
        prompt,
    )?;
    EventBus::new(session.session_dir.clone()).emit_session_status(
        &session.id,
        session.round,
        session.status.as_str(),
    )?;
    Ok(Json(session))
}

async fn get_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<Session>, ApiError> {
    Ok(Json(state.session_manager.use_session(&session_id)?))
}

async fn get_session_status(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionStatusSnapshot>, ApiError> {
    let session = state.session_manager.use_session(&session_id)?;
    let token_usage = EventBus::new(session.session_dir.clone())
        .latest_token_usage()?
        .unwrap_or_default();
    Ok(Json(SessionStatusSnapshot {
        session,
        input_tokens: token_usage.input_tokens,
        output_tokens: token_usage.output_tokens,
        cache_hit_rate: token_usage.cache_hit_rate,
        remaining_context: token_usage.remaining_context,
    }))
}

async fn delete_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let audit_id = state.session_manager.delete_session(&session_id, "api")?;
    Ok(Json(DeleteResponse { audit_id }))
}

async fn restore_session(
    State(state): State<ApiState>,
    Path(key): Path<String>,
) -> Result<Json<Session>, ApiError> {
    Ok(Json(state.session_manager.restore_session(&key)?))
}

async fn list_events(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<IpcEvent>>, ApiError> {
    let session = state.session_manager.use_session(&session_id)?;
    Ok(Json(EventBus::new(session.session_dir).list_events()?))
}

async fn list_working_memories(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<WorkingMemoryEntry>>, ApiError> {
    Ok(Json(
        state.session_manager.list_working_memories(&session_id)?,
    ))
}

async fn list_agents(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<AgentInstance>>, ApiError> {
    Ok(Json(
        state.session_manager.list_agent_instances(&session_id)?,
    ))
}

async fn list_tool_calls(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ToolCallRecord>>, ApiError> {
    Ok(Json(state.session_manager.list_tool_calls(&session_id)?))
}

async fn list_tool_results(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ToolResultRecord>>, ApiError> {
    Ok(Json(
        state
            .session_manager
            .list_tool_result_indexes(&session_id)?,
    ))
}

async fn get_tool_result_body(
    State(state): State<ApiState>,
    Path((session_id, tool_call_id)): Path<(String, String)>,
) -> Result<Json<ToolResultBodyResponse>, ApiError> {
    state.session_manager.use_session(&session_id)?;
    let output = state
        .session_manager
        .read_tool_result_by_call_id(&session_id, &tool_call_id)?;

    Ok(Json(ToolResultBodyResponse {
        tool_call_id,
        output,
    }))
}

async fn list_workspaces(
    State(state): State<ApiState>,
) -> Result<Json<Vec<WorkspaceDirectory>>, ApiError> {
    Ok(Json(state.session_manager.list_workspaces()?))
}

async fn get_workspace(
    State(state): State<ApiState>,
    Path(key): Path<String>,
) -> Result<Json<WorkspaceDirectory>, ApiError> {
    let workspace = state
        .session_manager
        .list_workspaces()?
        .into_iter()
        .find(|item| item.id == key || item.project_name == key || item.project_path == key)
        .ok_or_else(|| anyhow!("未找到工作区：{}", key))?;
    Ok(Json(workspace))
}

async fn delete_workspace(
    State(state): State<ApiState>,
    Path(key): Path<String>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let audit_id = state.session_manager.delete_workspace(&key, "api")?;
    Ok(Json(DeleteResponse { audit_id }))
}

async fn restore_workspace(
    State(state): State<ApiState>,
    Path(key): Path<String>,
) -> Result<Json<WorkspaceDirectory>, ApiError> {
    Ok(Json(state.session_manager.restore_workspace(&key)?))
}

async fn list_audits(State(state): State<ApiState>) -> Result<Json<Vec<DeletionAudit>>, ApiError> {
    Ok(Json(state.session_manager.list_deletion_audits(None)?))
}

async fn get_recovery_log(
    State(state): State<ApiState>,
) -> Result<Json<RecoveryLogResponse>, ApiError> {
    let path = state.settings.data_root.join("recovery.log");
    let content =
        read_optional_utf8(&path)?.ok_or_else(|| anyhow!("恢复日志不存在：{}", path.display()))?;
    Ok(Json(RecoveryLogResponse { content }))
}

struct ApiError(anyhow::Error);

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self(value.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = if self.0.to_string().contains("未找到")
            || self.0.to_string().contains("不存在")
        {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        let body = Json(serde_json::json!({
            "error": self.0.to_string()
        }));
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::ApiApp;

    #[tokio::test]
    async fn should_return_health_create_delete_restore_and_list_workspaces() {
        let workspace = PathBuf::from(format!(
            "target/test_api_workspace_{}",
            uuid::Uuid::new_v4()
        ));
        let app = ApiApp::new(workspace).expect("创建 API 应用失败");
        let router = app.router();

        let health = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("请求 health 失败");
        assert_eq!(health.status(), StatusCode::OK);

        let create = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"api-demo"}"#))
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

        let tool_results = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/{}/tool-results", session_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("请求 tool-results 失败");
        assert_eq!(tool_results.status(), StatusCode::OK);

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

        let delete = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/sessions/{}", session_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("删除会话请求失败");
        assert_eq!(delete.status(), StatusCode::OK);
        let delete_body = axum::body::to_bytes(delete.into_body(), usize::MAX)
            .await
            .expect("读取删除响应失败");
        let deleted: serde_json::Value =
            serde_json::from_slice(&delete_body).expect("解析删除响应失败");
        let audit_id = deleted
            .get("audit_id")
            .and_then(serde_json::Value::as_str)
            .expect("缺少 audit_id")
            .to_string();

        let restore = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/{}/restore", audit_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("恢复会话请求失败");
        assert_eq!(restore.status(), StatusCode::OK);

        let tool_result_body = router
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/{}/tool-results/not-found", session_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("请求 tool-result body 失败");
        assert_eq!(tool_result_body.status(), StatusCode::NOT_FOUND);
    }
}
