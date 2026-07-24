//! Turn a finished transcript into a markdown meeting summary via Ollama.

use anyhow::Result;

use super::llm::LlmClient;

const SYSTEM_PROMPT: &str = "You are a meticulous meeting-notes assistant. Summarize the transcript into markdown with sections: ## TL;DR (2-3 sentences), ## Key points, ## Decisions, ## Action items (checkbox list with owner names from the transcript), ## Open questions. Write in English; keep short Chinese quotes verbatim where the original wording matters. Do not invent facts not in the transcript.";

/// Summarizes `transcript_md` into markdown (`## TL;DR` / `## Key points` /
/// `## Decisions` / `## Action items` / `## Open questions`) using the given
/// Ollama client.
pub fn summarize(llm: &LlmClient, transcript_md: &str) -> Result<String> {
    llm.chat(SYSTEM_PROMPT, transcript_md)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn summarize_sends_exact_system_prompt_and_returns_markdown() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .body_includes(SYSTEM_PROMPT)
                .body_includes("Alice: let's ship on Friday.");
            then.status(200).json_body(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "## TL;DR\nShipping Friday.\n\n## Key points\n- x\n\n## Decisions\n- y\n\n## Action items\n- [ ] Alice: ship it\n\n## Open questions\n- none"
                }
            }));
        });

        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        };

        let out = summarize(&llm, "Alice: let's ship on Friday.").unwrap();

        assert!(out.starts_with("## TL;DR"));
        assert!(out.contains("## Key points"));
        assert!(out.contains("## Decisions"));
        assert!(out.contains("## Action items"));
        assert!(out.contains("## Open questions"));
        m.assert();
    }
}
