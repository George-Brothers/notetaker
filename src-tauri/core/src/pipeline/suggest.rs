//! Suggest which existing task a finished recording belongs to, using the
//! meeting summary and the user's current task list.

use anyhow::{Context, Result};

use super::llm::LlmClient;

const SYSTEM_PROMPT: &str = "Pick which task this recording belongs to. Reply with ONLY a JSON object {\"task\": string, \"confidence\": number 0-1}. The task MUST be one of the provided list, or \"\" if none fit.";

/// Confidence below this is treated as "not sure enough" -> `None` (Unsorted).
const CONFIDENCE_THRESHOLD: f32 = 0.6;

/// The model's pick, if any. `task: None` means Unsorted (either the model
/// was unsure or it named something outside the user's task list).
pub struct Suggestion {
    pub task: Option<String>,
    pub confidence: f32,
}

#[derive(serde::Deserialize)]
struct RawSuggestion {
    task: String,
    confidence: f32,
}

pub fn suggest_task(llm: &LlmClient, summary: &str, tasks: &[String]) -> Result<Suggestion> {
    let task_list = tasks.join(", ");
    let user = format!("Task list: [{task_list}]\n\nSummary:\n{summary}");

    let reply = llm.chat(SYSTEM_PROMPT, &user)?;

    // Parse leniently: the model sometimes wraps JSON in a ```json fence.
    let json_text = reply
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let raw: RawSuggestion = serde_json::from_str(json_text)
        .with_context(|| format!("ollama returned a suggestion we couldn't parse: {reply}"))?;

    let task = if raw.confidence >= CONFIDENCE_THRESHOLD && tasks.iter().any(|t| t == &raw.task) {
        Some(raw.task)
    } else {
        None
    };

    Ok(Suggestion {
        task,
        confidence: raw.confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn tasks() -> Vec<String> {
        vec!["Accounting 302".to_string(), "Notetaker app".to_string()]
    }

    #[test]
    fn suggest_parses_json_and_applies_threshold() {
        let server = MockServer::start();

        // Case 1: confidence below threshold -> rejected to None even
        // though the task itself is valid.
        let mut low = server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": r#"{"task": "Accounting 302", "confidence": 0.41}"#
                }
            }));
        });
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        };
        let s = suggest_task(&llm, "summary text", &tasks()).unwrap();
        assert_eq!(s.task, None, "0.41 confidence is below the 0.6 threshold");
        assert!((s.confidence - 0.41).abs() < 1e-6);
        low.delete();

        // Case 2: confidence at/above threshold and task is in the list ->
        // accepted.
        let mut high = server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": r#"{"task": "Accounting 302", "confidence": 0.9}"#
                }
            }));
        });
        let s = suggest_task(&llm, "summary text", &tasks()).unwrap();
        assert_eq!(s.task, Some("Accounting 302".to_string()));
        assert!((s.confidence - 0.9).abs() < 1e-6);
        high.delete();

        // Case 3: high confidence but the model named a task NOT in the
        // provided list -> never invent tasks, so None.
        let mut invented = server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": r#"{"task": "Made Up Task", "confidence": 0.95}"#
                }
            }));
        });
        let s = suggest_task(&llm, "summary text", &tasks()).unwrap();
        assert_eq!(
            s.task, None,
            "a task outside the provided list must never be invented"
        );
        invented.delete();
    }
}
