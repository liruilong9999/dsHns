//! MCP 客户端实现。

use std::env;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use crate::utils::time::now_rfc3339;

/// MCP 服务端配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 服务端标识。
    pub id: String,
    /// 服务端名称。
    pub name: String,
    /// 基础地址。
    pub base_url: String,
    /// 是否启用。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 能力查询端点。
    pub capabilities_endpoint: Option<String>,
    /// 工具调用基础端点。
    pub tools_endpoint: Option<String>,
    /// 可选 API Key 环境变量名。
    pub api_key_env: Option<String>,
}

/// MCP 客户端状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientState {
    /// 服务端标识。
    pub server_id: String,
    /// 当前状态。
    pub status: String,
    /// 能力摘要。
    pub capabilities_json: String,
    /// 最近错误。
    pub last_error: String,
    /// 更新时间。
    pub updated_at: String,
}

/// MCP 管理器。
pub struct McpClientManager {
    /// 工作区根目录。
    workspace_root: PathBuf,
    /// 当前会话目录。
    session_dir: PathBuf,
    /// HTTP 客户端。
    client: Client,
}

impl McpClientManager {
    /// 创建 MCP 管理器。
    pub fn new(workspace_root: PathBuf, session_dir: PathBuf) -> Self {
        Self {
            workspace_root,
            session_dir,
            client: Client::new(),
        }
    }

    /// 发现 MCP 服务端。
    pub fn discover_servers(&self) -> Result<Vec<McpServerConfig>> {
        let path = self.servers_file_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = read_optional_utf8(&path)?
            .ok_or_else(|| anyhow!("读取 MCP 配置文件失败：{}", path.display()))?;
        let servers: Vec<McpServerConfig> = serde_json::from_str(&content)
            .with_context(|| format!("解析 MCP 配置文件失败：{}", path.display()))?;
        Ok(servers
            .into_iter()
            .filter(|server| server.enabled)
            .collect())
    }

    /// 连接 MCP 服务端。
    pub async fn connect_server(&self, server_id: &str) -> Result<McpClientState> {
        let server = self.find_server(server_id)?;
        let url = server
            .capabilities_endpoint
            .clone()
            .unwrap_or_else(|| format!("{}/capabilities", server.base_url.trim_end_matches('/')));
        let response = self
            .authorized_request(self.client.get(url), &server)?
            .send()
            .await
            .context("连接 MCP 服务端失败")?
            .error_for_status()
            .context("MCP 服务端返回失败状态")?
            .json::<Value>()
            .await
            .context("解析 MCP 能力响应失败")?;

        let state = McpClientState {
            server_id: server.id,
            status: "connected".to_string(),
            capabilities_json: serde_json::to_string(&response)?,
            last_error: String::new(),
            updated_at: now_rfc3339(),
        };
        self.persist_state(&state)?;
        Ok(state)
    }

    /// 调用远程 MCP 工具。
    pub async fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let server = self.find_server(server_id)?;
        let base = server
            .tools_endpoint
            .clone()
            .unwrap_or_else(|| format!("{}/tools", server.base_url.trim_end_matches('/')));
        let url = format!("{}/{}", base.trim_end_matches('/'), tool_name);
        self.authorized_request(self.client.post(url), &server)?
            .json(&arguments)
            .send()
            .await
            .context("调用 MCP 工具失败")?
            .error_for_status()
            .context("MCP 工具返回失败状态")?
            .json::<Value>()
            .await
            .context("解析 MCP 工具响应失败")
    }

    fn find_server(&self, server_id: &str) -> Result<McpServerConfig> {
        self.discover_servers()?
            .into_iter()
            .find(|server| server.id == server_id)
            .ok_or_else(|| anyhow!("未找到 MCP 服务端：{}", server_id))
    }

    fn authorized_request(
        &self,
        builder: reqwest::RequestBuilder,
        server: &McpServerConfig,
    ) -> Result<reqwest::RequestBuilder> {
        if let Some(env_name) = server.api_key_env.as_ref() {
            let token = env::var(env_name)
                .map_err(|_| anyhow!("缺少 MCP 服务端所需环境变量：{}", env_name))?;
            Ok(builder.bearer_auth(token))
        } else {
            Ok(builder)
        }
    }

    fn persist_state(&self, state: &McpClientState) -> Result<()> {
        let path = self.clients_file_path();
        if let Some(parent) = path.parent() {
            ensure_directory(parent)?;
        }

        let original = read_optional_utf8(&path)?.unwrap_or_else(|| "[]".to_string());
        let mut states: Vec<McpClientState> = serde_json::from_str(&original).unwrap_or_default();
        if let Some(index) = states
            .iter()
            .position(|item| item.server_id == state.server_id)
        {
            states[index] = state.clone();
        } else {
            states.push(state.clone());
        }
        write_utf8(&path, &serde_json::to_string_pretty(&states)?)
    }

    fn servers_file_path(&self) -> PathBuf {
        self.workspace_root.join(".dshns").join("mcp_servers.json")
    }

    fn clients_file_path(&self) -> PathBuf {
        self.session_dir
            .join(".tools")
            .join("mcp")
            .join("clients.json")
    }
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::utils::fs::write_utf8;

    use super::McpClientManager;

    #[test]
    fn should_discover_servers_from_file() {
        let workspace = PathBuf::from("target/test_mcp_workspace");
        let session = workspace.join("session");
        let path = workspace.join(".dshns").join("mcp_servers.json");
        write_utf8(
            &path,
            r#"[{"id":"demo","name":"demo","base_url":"http://localhost:8080","enabled":true}]"#,
        )
        .expect("写入 MCP 配置失败");

        let manager = McpClientManager::new(workspace, session);
        let servers = manager.discover_servers().expect("发现服务端失败");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "demo");
    }
}
