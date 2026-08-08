//! Turn a finished transcript — and whatever the user typed during the
//! meeting — into a markdown summary via Ollama.
//!
//! The user's notes are the important input here, not a decoration. Someone who
//! wrote "15%?" during a call has told you what mattered about it far more
//! precisely than the transcript does, so when notes exist the model is asked to
//! *expand on them* rather than summarize the meeting from scratch. That is the
//! difference between notes that read like yours and notes that read like a
//! transcript robot's.
//!
//! The section list comes from the recording's [template](crate::templates), so
//! the same transcript produces study notes or a client-call record depending on
//! what the user picked.

use anyhow::Result;

use super::llm::{KeepAlive, LlmClient};
use crate::notes;
use crate::templates;

/// The instruction wrapped around every template's section list.
const BASE: &str = "You are a meticulous meeting-notes assistant. Write markdown with exactly these sections, in this order:";

/// Rules appended after the sections, for every template.
const RULES: &str = "Write in English; keep short Chinese quotes verbatim where the original wording matters. Do not invent facts that are not in the material. If a section has nothing in it, keep the heading and write \"Nothing noted.\" under it.";

/// The extra instruction used when the user typed notes during the recording.
const WITH_NOTES: &str = "The user's own notes are provided first. They are the point: treat each of their lines as a heading of interest, expand it using the transcript, correct it where the transcript disagrees, and keep their wording where you can. Cover anything important they missed, but never drop something they bothered to write down.";

/// Summarizes a recording into markdown shaped by its template.
///
/// `notes_md` may be empty — most recordings have no typed notes, and that is
/// the plain "summarize the transcript" path. `template_id` may be `None` or
/// name a template this build no longer has; either way a summary is produced
/// (see [`templates::find`]), because refusing to summarize over a template id
/// is never the right trade.
pub fn summarize(
    llm: &LlmClient,
    transcript_md: &str,
    notes_md: &str,
    template_id: Option<&str>,
    templates: &[templates::Template],
) -> Result<String> {
    summarize_with_keep_alive(
        llm,
        transcript_md,
        notes_md,
        template_id,
        templates,
        KeepAlive::Final,
    )
}

/// Summarizes with an explicit Ollama lifetime. Processing batches use the
/// warm setting for this first call and release the model on their final call.
pub fn summarize_with_keep_alive(
    llm: &LlmClient,
    transcript_md: &str,
    notes_md: &str,
    template_id: Option<&str>,
    templates: &[templates::Template],
    keep_alive: KeepAlive,
) -> Result<String> {
    let has_notes = notes::has_content(notes_md);
    llm.chat_with_keep_alive(
        &system_prompt(templates, template_id, has_notes),
        &user_content(transcript_md, notes_md),
        keep_alive,
    )
}

/// Builds the system prompt. Separate so the composition is testable without a
/// server — this is where a template silently failing to reach the model would
/// hide.
fn system_prompt(templates: &[templates::Template], template_id: Option<&str>, has_notes: bool) -> String {
    let template = templates::find(templates, template_id);
    let mut out = format!("{BASE}\n{}\n\n{RULES}", template.sections);
    if has_notes {
        out.push_str("\n\n");
        out.push_str(WITH_NOTES);
    }
    out
}

fn user_content(transcript_md: &str, notes_md: &str) -> String {
    if notes::has_content(notes_md) {
        format!(
            "=== The user's own notes, typed during the meeting ===\n{}\n\n=== Transcript ===\n{}",
            notes_md.trim(),
            transcript_md
        )
    } else {
        transcript_md.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn llm(server: &MockServer) -> LlmClient {
        LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        }
    }

    fn test_templates() -> Vec<templates::Template> {
        templates::defaults()
    }

    fn reply(server: &MockServer, content: &str) {
        let content = content.to_string();
        server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": content }
            }));
        });
    }

    #[test]
    fn summarize_sends_the_default_sections_and_returns_markdown() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .body_includes("## TL;DR")
                .body_includes("## Action items")
                .body_includes("Alice: let's ship on Friday.");
            then.status(200).json_body(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "## TL;DR\nShipping Friday.\n\n## Action items\n- [ ] Alice: ship it"
                }
            }));
        });

        let templates = test_templates();
        let out = summarize(&llm(&server), "Alice: let's ship on Friday.", "", None, &templates).unwrap();

        assert!(out.starts_with("## TL;DR"));
        assert!(out.contains("## Action items"));
        m.assert();
    }

    /// The whole point of the notepad: what the user wrote has to reach the
    /// model, and the model has to be told to build on it.
    #[test]
    fn the_users_notes_reach_the_model_with_the_expand_instruction() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .body_includes("15%?")
                .body_includes("never drop something they bothered to write down");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "## TL;DR\n15% agreed." }
            }));
        });

        let templates = test_templates();
        summarize(&llm(&server), "transcript", "- pricing\n- 15%?", None, &templates).unwrap();
        m.assert();
    }

    #[test]
    fn a_recording_with_no_notes_does_not_get_the_notes_instruction() {
        let templates = test_templates();
        let prompt = system_prompt(&templates, None, false);
        assert!(
            !prompt.contains("user's own notes"),
            "an unused instruction is noise: {prompt}"
        );
        // And the user turn is just the transcript, unwrapped.
        assert_eq!(
            user_content("just the transcript", "  \n "),
            "just the transcript"
        );
    }

    #[test]
    fn whitespace_only_notes_count_as_no_notes() {
        let templates = test_templates();
        assert!(!system_prompt(&templates, None, false).contains("user's own notes"));
        assert_eq!(user_content("t", "\n\t  \n"), "t");
    }

    /// A template that did not reach the model would produce the default shape
    /// while the UI claimed otherwise — silent, and exactly the failure worth a
    /// test.
    #[test]
    fn the_chosen_templates_sections_are_the_ones_sent() {
        let templates = test_templates();
        let lecture = system_prompt(&templates, Some("lecture"), false);
        assert!(lecture.contains("Likely exam material"), "{lecture}");
        assert!(!lecture.contains("What the client asked for"));

        let client = system_prompt(&templates, Some("client_call"), false);
        assert!(client.contains("What the client asked for"), "{client}");
        assert!(!client.contains("Likely exam material"));
    }

    #[test]
    fn every_template_produces_a_prompt_with_its_sections_and_the_rules() {
        let templates = test_templates();
        for t in &templates {
            let prompt = system_prompt(&templates, Some(&t.id), false);
            assert!(prompt.contains(&t.sections), "{} lost its sections", t.id);
            assert!(prompt.contains(RULES), "{} lost the shared rules", t.id);
        }
    }

    /// A `meta.json` naming a template this build dropped must still summarize.
    #[test]
    fn an_unknown_template_id_still_summarizes_in_the_default_shape() {
        let server = MockServer::start();
        reply(&server, "## TL;DR\nfine");
        let out = summarize(
            &llm(&server),
            "transcript",
            "",
            Some("template_from_the_future"),
            &test_templates(),
        )
        .unwrap();
        assert!(out.contains("TL;DR"));
        let templates = test_templates();
        assert!(system_prompt(&templates, Some("template_from_the_future"), false)
            .contains(&templates[0].sections));
    }

    /// An empty section is a heading with "Nothing noted." under it, not a
    /// missing heading — otherwise the UI's section navigation changes shape
    /// from one recording to the next.
    #[test]
    fn the_rules_tell_the_model_to_keep_empty_headings() {
        assert!(RULES.contains("Nothing noted."), "{RULES}");
    }
}
