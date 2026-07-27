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

/// The model's raw reply. Both fields tolerate being missing or `null`: this
/// is the last stage of a long pipeline, and a local model answering
/// `{"task": null}` — a perfectly reasonable way to say "none of these fit" —
/// used to fail the whole run and throw away an otherwise good transcript.
/// A suggestion we cannot read means Unsorted, never a lost recording.
#[derive(serde::Deserialize)]
struct RawSuggestion {
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
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

    let named = raw.task.unwrap_or_default();
    let task = if raw.confidence >= CONFIDENCE_THRESHOLD && tasks.iter().any(|t| t == &named) {
        Some(named)
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

    /// A local model saying "none of these fit" as `null` rather than `""` —
    /// or omitting a field entirely — must land in Unsorted, not fail the run.
    /// This is the last stage of the pipeline, so an error here would discard
    /// a finished transcript and summary over a suggestion nobody needed.
    #[test]
    fn an_unreadable_suggestion_means_unsorted_not_a_lost_recording() {
        let server = MockServer::start();
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".to_string(),
        };

        for reply in [
            r#"{"task": null, "confidence": 0.9}"#,
            r#"{"confidence": 0.9}"#,
            r#"{"task": "Accounting 302"}"#,
            r#"{}"#,
        ] {
            let mut mock = server.mock(|when, then| {
                when.method(POST).path("/api/chat");
                then.status(200).json_body(serde_json::json!({
                    "message": { "role": "assistant", "content": reply }
                }));
            });
            let s = suggest_task(&llm, "summary text", &tasks())
                .unwrap_or_else(|e| panic!("{reply} should not fail the run: {e:#}"));
            assert_eq!(s.task, None, "{reply} should land in Unsorted");
            mock.delete();
        }
    }
}
