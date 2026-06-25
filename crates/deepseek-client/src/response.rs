use serde::Deserialize;
use dshns_core::message::{Usage, ToolCallRef};

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ResponseMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseMessage {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallRef>>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta { delta: String },
    ToolCallStart { id: String, name: String },
    ToolCallArgDelta { id: String, delta: String },
    ToolCallComplete { id: String, name: String, arguments: serde_json::Value },
    Finished { finish_reason: String, usage: Usage },
    Error { message: String },
}

/// 解析一行 SSE data
pub fn parse_sse_line(line: &str) -> Option<StreamEvent> {
    let line = line.trim();
    if line.is_empty() { return None; }
    if line == "data: [DONE]" {
        return Some(StreamEvent::Finished {
            finish_reason: "stop".into(),
            usage: Usage::default(),
        });
    }
    if !line.starts_with("data: ") { return None; }
    let json_str = &line[6..];
    let chunk: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let choices = chunk["choices"].as_array()?;

    if choices.is_empty() {
        let reason = chunk["choices"][0]["finish_reason"].as_str().unwrap_or("stop").to_string();
        let usage = serde_json::from_value(chunk["usage"].clone()).unwrap_or_default();
        return Some(StreamEvent::Finished { finish_reason: reason, usage });
    }

    let delta = &choices[0]["delta"];

    if let Some(tool_calls) = delta["tool_calls"].as_array() {
        for tc in tool_calls {
            if let (Some(id), Some(name)) = (tc["id"].as_str(), tc["function"]["name"].as_str()) {
                return Some(StreamEvent::ToolCallStart { id: id.into(), name: name.into() });
            }
            if let Some(args) = tc["function"]["arguments"].as_str() {
                if !args.is_empty() {
                    return Some(StreamEvent::ToolCallArgDelta {
                        id: tc["index"].as_u64().map(|i| format!("pending_{}", i)).unwrap_or_default(),
                        delta: args.into(),
                    });
                }
            }
        }
    }

    if let Some(content) = delta["content"].as_str() {
        if !content.is_empty() {
            return Some(StreamEvent::TextDelta { delta: content.into() });
        }
    }

    if let Some(reason) = choices[0]["finish_reason"].as_str() {
        if reason != "null" {
            let usage = serde_json::from_value(chunk["usage"].clone()).unwrap_or_default();
            return Some(StreamEvent::Finished { finish_reason: reason.into(), usage });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_delta() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#;
        match parse_sse_line(line) {
            Some(StreamEvent::TextDelta { delta }) => assert_eq!(delta, "Hello"),
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_call_start() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":""}}]}}]}"#;
        match parse_sse_line(line) {
            Some(StreamEvent::ToolCallStart { id, name }) => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
            }
            other => panic!("expected ToolCallStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_done() {
        match parse_sse_line("data: [DONE]") {
            Some(StreamEvent::Finished { .. }) => {},
            other => panic!("expected Finished, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_empty() {
        assert!(parse_sse_line("").is_none());
    }
}
