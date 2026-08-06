//! Minimal client for a local Ollama server's `/api/chat` endpoint.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{bail, Context, Result};
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

/// The application is local-only. Keep this check next to the HTTP client so
/// every caller, including dictation cleanup, is protected when settings.json
/// is edited by hand.
pub fn is_local_ollama_url(base_url: &str) -> bool {
    let Some((scheme, authority)) = base_url.trim().split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let authority = authority.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host.eq_ignore_ascii_case("localhost") || matches!(host, "127.0.0.1" | "::1")
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    stream: bool,
    /// Keep Ollama's model resident through the other calls in one recording,
    /// then send numeric zero on the final call so Ollama may release it.
    keep_alive: serde_json::Value,
}

/// Lifetime requested for one Ollama chat call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepAlive {
    /// Keep the summary model warm while the rest of a recording's batch runs.
    Batch,
    /// This is the batch's final call; allow Ollama to unload immediately.
    Final,
}

impl KeepAlive {
    fn json_value(self) -> serde_json::Value {
        match self {
            Self::Batch => serde_json::json!("10m"),
            Self::Final => serde_json::json!(0),
        }
    }
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

#[derive(Deserialize)]
struct StreamChatResponse {
    #[serde(default)]
    message: Option<ChatResponseMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    error: Option<String>,
}

impl LlmClient {
    /// Sends one system+user turn to `{base_url}/api/chat` (non-streaming)
    /// and returns the assistant's reply text. A standalone call is treated as
    /// final; recording batches use [`Self::chat_with_keep_alive`] explicitly.
    pub fn chat(&self, system: &str, user: &str) -> Result<String> {
        self.chat_with_keep_alive(system, user, KeepAlive::Final)
    }

    /// Sends a chat call with the Ollama lifetime appropriate to its place in
    /// a batch.
    pub fn chat_with_keep_alive(
        &self,
        system: &str,
        user: &str,
        keep_alive: KeepAlive,
    ) -> Result<String> {
        if !is_local_ollama_url(&self.base_url) {
            bail!("local-only Ollama rejected non-local address {}", self.base_url);
        }
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
            keep_alive: keep_alive.json_value(),
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

    /// Streams an Ollama answer as newline-delimited JSON. The callback is
    /// invoked for each content fragment, so a caller can render the answer
    /// without waiting for the model's final `done` line.
    pub fn chat_stream<F>(&self, system: &str, user: &str, mut on_token: F) -> Result<String>
    where
        F: FnMut(&str),
    {
        if !is_local_ollama_url(&self.base_url) {
            bail!("local-only Ollama rejected non-local address {}", self.base_url);
        }
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
            stream: true,
            keep_alive: KeepAlive::Batch.json_value(),
        };

        let response = ureq::post(&url)
            .config()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .build()
            .send_json(&body)
            .with_context(|| format!("could not reach ollama at {url}"))?;
        let reader = BufReader::new(response.into_body().into_reader());
        let mut answer = String::new();
        let mut done = false;

        for line in reader.lines() {
            let line = line.context("reading ollama's streamed answer")?;
            let parsed: StreamChatResponse = serde_json::from_str(line.trim())
                .context("ollama returned a streamed answer we couldn't parse")?;
            if let Some(error) = parsed.error {
                anyhow::bail!("ollama could not answer: {error}");
            }
            if let Some(message) = parsed.message {
                if !message.content.is_empty() {
                    on_token(&message.content);
                    answer.push_str(&message.content);
                }
            }
            if parsed.done {
                done = true;
                break;
            }
        }

        if !done {
            anyhow::bail!("ollama ended the streamed answer before it finished");
        }
        Ok(answer)
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
                .json_body_includes(r#"{"stream": false, "keep_alive": 0}"#);
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
    fn batch_calls_keep_ollama_warm_but_the_final_call_sends_zero() {
        let server = MockServer::start();
        let warm = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .json_body_includes(r#"{"keep_alive": "10m"}"#);
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "warm" }
            }));
        });
        let final_call = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .json_body_includes(r#"{"keep_alive": 0}"#);
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "done" }
            }));
        });
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        };

        assert_eq!(
            llm.chat_with_keep_alive("s", "u", KeepAlive::Batch)
                .unwrap(),
            "warm"
        );
        assert_eq!(llm.chat("s", "u").unwrap(), "done");
        warm.assert();
        final_call.assert();
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

    #[test]
    fn chat_stream_posts_streaming_shape_and_emits_tokens_in_order() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .json_body_includes(r#"{"stream": true, "keep_alive": "10m"}"#);
            then.status(200).body(
                "{\"message\":{\"content\":\"Hello\"},\"done\":false}\n{\"message\":{\"content\":\" there\"},\"done\":false}\n{\"done\":true}\n",
            );
        });

        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:1.7b".into(),
        };
        let mut tokens = Vec::new();
        let answer = llm
            .chat_stream("system", "question", |token| tokens.push(token.to_string()))
            .unwrap();
        assert_eq!(tokens, ["Hello", " there"]);
        assert_eq!(answer, "Hello there");
        m.assert();
    }

    #[test]
    fn non_local_ollama_addresses_are_rejected_before_network_access() {
        let llm = LlmClient {
            base_url: "https://ollama.example.test".into(),
            model: "small".into(),
        };
        let error = llm.chat("system", "user").unwrap_err().to_string();
        assert!(error.contains("local-only Ollama"));
    }
}
