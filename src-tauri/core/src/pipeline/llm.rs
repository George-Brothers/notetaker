//! Minimal client for a local Ollama server's `/api/chat` endpoint.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Cap on establishing the TCP connection only (not the response, which for
/// a local LLM can legitimately take a while to generate). Keeps "Ollama is
/// not running" a fast, clean failure instead of a multi-minute hang.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Talks to a local Ollama server. No default is provided: callers set
/// `base_url` (normally `"http://localhost:11434"`) and `model` explicitly.
pub struct LlmClient {
    pub base_url: String,
    pub model: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

impl LlmClient {
    /// Sends one system+user turn to `{base_url}/api/chat` (non-streaming)
    /// and returns the assistant's reply text.
    pub fn chat(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/api/chat", self.base_url);
        let body = ChatRequest {
            model: &self.model,
            messages: [
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            stream: false,
        };

        let mut response = ureq::post(&url)
            .config()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .build()
            .send_json(&body)
            .with_context(|| format!("could not reach ollama at {url}"))?;

        let parsed: ChatResponse = response
            .body_mut()
            .read_json()
            .context("ollama returned a response we couldn't parse")?;

        Ok(parsed.message.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn chat_posts_ollama_shape_and_returns_content() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .json_body_includes(r#"{"stream": false}"#);
            then.status(200).json_body(serde_json::json!({
                "message": {"role": "assistant", "content": "## TL;DR\nhi"}
            }));
        });

        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        };

        assert!(llm.chat("sys", "user").unwrap().contains("TL;DR"));
        m.assert();
    }

    #[test]
    fn ollama_down_is_a_clean_error() {
        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".into(),
            model: "x".into(),
        };

        let e = llm.chat("s", "u").unwrap_err().to_string();
        assert!(
            e.to_lowercase().contains("ollama"),
            "error must name ollama for the UI: {e}"
        );
    }
}
