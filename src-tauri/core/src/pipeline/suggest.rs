//! Suggest which existing task a finished recording belongs to, using the
//! meeting summary and the user's current task list — and a better title than
//! the timestamp the recording was created with.

use anyhow::{Context, Result};

use super::llm::LlmClient;

const SYSTEM_PROMPT: &str = "Pick which task this recording belongs to. Reply with ONLY a JSON object {\"task\": string, \"confidence\": number 0-1}. The task MUST be one of the provided list, or \"\" if none fit.";

const TITLE_PROMPT: &str = "Write a short title for this recording: what it was about, 3 to 8 words, no quotes, no trailing period, no date, no time. Reply with ONLY the title on one line.";

/// A title longer than this is a sentence, not a title. Long enough for
/// "Accounting 302 midterm review with the study group", short enough that the
/// sidebar never has to truncate mid-word.
const MAX_TITLE_CHARS: usize = 70;

/// Suggests a better title for a recording from its summary.
///
/// Returns `None` rather than erroring when the model gives back something
/// unusable. A title is a nicety offered for one-click acceptance next to the
/// auto-generated one — never worth failing a run that already produced a good
/// transcript, and never worth applying automatically. The user accepts it, the
/// same way they accept a suggested task.
pub fn suggest_title(llm: &LlmClient, summary: &str) -> Result<Option<String>> {
    let reply = llm.chat(TITLE_PROMPT, summary)?;
    Ok(clean_title(&reply))
}

/// Turns a model's reply into a usable title, or `None`.
///
/// Separated from the call so the cleanup is testable without a mock server —
/// it is where all the real failure modes live. Small models like to answer a
/// "reply with only X" instruction with a code fence, a "Title:" label, or a
/// short essay.
fn clean_title(reply: &str) -> Option<String> {
    let line = reply
        .trim()
        .trim_start_matches("```markdown")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .lines()
        // The first line with anything on it: a model that adds a preamble
        // usually puts the title on its own line after it.
        .find(|l| !l.trim().is_empty())?
        .trim();

    // Strip a "Title:" style label if there is one.
    let line = match line.split_once(':') {
        Some((label, rest))
            if label.trim().eq_ignore_ascii_case("title") && !rest.trim().is_empty() =>
        {
            rest.trim()
        }
        _ => line,
    };

    let cleaned = line
        .trim_matches(|c| c == '"' || c == '\'' || c == '*' || c == '#')
        .trim()
        .trim_end_matches('.')
        .trim();

    if cleaned.is_empty() || cleaned.chars().count() > MAX_TITLE_CHARS {
        return None;
    }
    // Path separators would make the accepted title and the folder it creates
    // disagree about where the recording lives. `Store` strips the rest of the
    // filesystem-hostile set from the *directory* name while `meta.title` keeps
    // it, which is fine — a title reading "Q3 planning: budget" is a good
    // title. A title containing a slash is not.
    if cleaned.contains(['/', '\\', '\n']) {
        return None;
    }
    Some(cleaned.to_string())
}

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

    // --- suggested titles -------------------------------------------------

    #[test]
    fn a_clean_reply_is_taken_as_the_title() {
        assert_eq!(
            clean_title("Accounting 302 midterm review"),
            Some("Accounting 302 midterm review".to_string())
        );
    }

    /// Everything a small model does when told "reply with ONLY the title".
    #[test]
    fn the_usual_model_decorations_are_stripped() {
        for (reply, expected) in [
            ("\"Q3 pricing call\"", "Q3 pricing call"),
            ("'Q3 pricing call'", "Q3 pricing call"),
            ("**Q3 pricing call**", "Q3 pricing call"),
            ("# Q3 pricing call", "Q3 pricing call"),
            ("Title: Q3 pricing call", "Q3 pricing call"),
            ("title: Q3 pricing call", "Q3 pricing call"),
            ("Q3 pricing call.", "Q3 pricing call"),
            ("```\nQ3 pricing call\n```", "Q3 pricing call"),
            ("  \n\nQ3 pricing call\n\nlet me know!", "Q3 pricing call"),
        ] {
            assert_eq!(
                clean_title(reply).as_deref(),
                Some(expected),
                "failed on {reply:?}"
            );
        }
    }

    /// A colon is legal in a title — `meta.title` keeps it and only the
    /// directory name strips it — so a real title must not be thrown away for
    /// having one.
    #[test]
    fn a_colon_inside_a_title_is_kept() {
        assert_eq!(
            clean_title("Q3 planning: budget review").as_deref(),
            Some("Q3 planning: budget review")
        );
    }

    #[test]
    fn an_unusable_reply_is_declined_rather_than_applied() {
        // Empty, whitespace, and a fence with nothing in it.
        for reply in ["", "   \n  ", "```\n```", "\"\""] {
            assert_eq!(clean_title(reply), None, "should decline {reply:?}");
        }
    }

    /// The model ignoring the instruction and writing a paragraph must not
    /// become a 400-character directory name.
    #[test]
    fn a_reply_that_is_a_sentence_not_a_title_is_declined() {
        let essay = "This recording is a discussion between George and the study group about \
                     the upcoming accounting midterm, covering the balance sheet";
        assert_eq!(clean_title(essay), None);
    }

    /// A slash would make the accepted title and the folder it creates
    /// disagree about where the recording lives.
    #[test]
    fn a_title_with_a_path_separator_is_declined() {
        assert_eq!(clean_title("Accounting 302 / 303 review"), None);
        assert_eq!(clean_title(r"Q3\Q4 planning"), None);
    }

    #[test]
    fn suggest_title_returns_the_cleaned_title() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST).path("/api/chat").body_includes("3 to 8");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "\"Q3 pricing call\"" }
            }));
        });
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        };
        assert_eq!(
            suggest_title(&llm, "## TL;DR\nWe agreed 15%.").unwrap(),
            Some("Q3 pricing call".to_string())
        );
        m.assert();
    }

    #[test]
    fn a_junk_title_reply_is_none_rather_than_an_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "```\n```" }
            }));
        });
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        };
        assert_eq!(suggest_title(&llm, "summary").unwrap(), None);
    }
}
