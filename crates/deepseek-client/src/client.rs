use std::time::Duration;
use reqwest::Client as HttpClient;
use dshns_core::config::ApiConfig;
use dshns_core::error::DshnsError;
use crate::request::ChatRequest;
use crate::response::{ChatResponse, StreamEvent, parse_sse_line};
use futures::Stream;
use std::pin::Pin;
use tokio::sync::mpsc;

pub struct DeepSeekClient {
    http: HttpClient,
    api_key: String,
    base_url: String,
    config: ApiConfig,
}

impl DeepSeekClient {
    pub fn new(api_key: String, config: ApiConfig) -> Result<Self, DshnsError> {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|e| DshnsError::Network(e.to_string()))?;
        Ok(Self { http, api_key, base_url: "https://api.deepseek.com".into(), config })
    }

    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, DshnsError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut r = req.clone();
        r.stream = false;
        let resp = self.http.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&r).send().await
            .map_err(|e| self.classify_error(e))?;
        self.handle_response(resp).await
    }

    pub async fn chat_stream(&self, req: &ChatRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, DshnsError>> + Send>>, DshnsError>
    {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut r = req.clone();
        r.stream = true;
        let resp = self.http.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&r).send().await
            .map_err(|e| self.classify_error(e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_http_error(status.as_u16(), &body));
        }

        let stream = resp.bytes_stream();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            use futures::StreamExt;
            tokio::pin!(stream);
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(b) => {
                        buf.push_str(&String::from_utf8_lossy(&b));
                        while let Some(pos) = buf.find('\n') {
                            let line = buf[..pos].to_string();
                            buf = buf[pos+1..].to_string();
                            if let Some(ev) = parse_sse_line(&line) {
                                if tx.send(Ok(ev)).is_err() { return; }
                            }
                        }
                    }
                    Err(e) => { let _ = tx.send(Err(DshnsError::Network(e.to_string()))); return; }
                }
            }
        });
        Ok(Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx)))
    }

    async fn handle_response(&self, resp: reqwest::Response) -> Result<ChatResponse, DshnsError> {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(self.classify_http_error(status.as_u16(), &body));
        }
        serde_json::from_str(&body).map_err(|e| DshnsError::SseParse(e.to_string()))
    }

    fn classify_http_error(&self, status: u16, body: &str) -> DshnsError {
        match status {
            401 => DshnsError::ApiAuth(body.into()),
            429 => DshnsError::ApiRateLimited(5),
            500..=599 => DshnsError::ApiServer(body.into()),
            _ => DshnsError::Other(format!("HTTP {}: {}", status, body)),
        }
    }

    fn classify_error(&self, e: reqwest::Error) -> DshnsError {
        if e.is_timeout() {
            DshnsError::Network("请求超时".into())
        } else if e.is_connect() {
            DshnsError::Network(format!("连接失败: {}", e))
        } else {
            DshnsError::Network(e.to_string())
        }
    }
}
