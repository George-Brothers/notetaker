//! "Ask this meeting": one question, answered from one recording.
//!
//! The user hits Cmd+J on a recording and types "what did we agree on
//! pricing". This sends that recording's own text to the local model and
//! returns the answer.
//!
//! Deliberately **stateless and single-turn**. There is no conversation
//! history, because the interesting failure mode of a chat over a transcript is
//! not "it forgot the last question", it is "it answered confidently about
//! something that was never said". A single turn with the whole recording in
//! front of it, and a prompt that insists on saying so when the answer is not
//! there, is the version that stays trustworthy. Follow-up questions work
//! because the whole transcript is re-sent each time — it is cheap, it is
//! local, and there is no context to go stale.

use anyhow::{bail, Result};

use super::llm::LlmClient;

const SYSTEM_PROMPT: &str = "You answer questions about ONE meeting, using only the notes and transcript provided. Rules: answer in 1-4 sentences; quote the transcript when the exact wording matters; if the answer is not in the material, say \"That doesn't come up in this recording\" and stop — never guess, never use outside knowledge, never invent a speaker or a number.";

/// How much of a recording's text is sent. A long lecture can run past a small
/// model's context window, and silently truncating in the *middle* is what
/// produces confident answers about the missing part. So the transcript is
/// trimmed from the front — keeping the end, where decisions and action items
/// live — and the model is told plainly that it happened.
const MAX_TRANSCRIPT_CHARS: usize = 48_000;

/// Answers `question` about one recording.
///
/// The user's own notes come first: they are short, dense, and the closest
/// thing to a statement of what mattered, so a model that reads them first
/// tends to answer in those terms.
pub fn ask(
    llm: &LlmClient,
    question: &str,
    notes_md: &str,
    summary_md: &str,
    transcript_md: &str,
) -> Result<String> {
    if question.trim().is_empty() {
        bail!("Type a question first.");
    }
    if transcript_md.trim().is_empty() && summary_md.trim().is_empty() {
        bail!("This recording hasn't been transcribed yet, so there's nothing to ask about.");
    }

    let answer = llm
        .chat(
            SYSTEM_PROMPT,
            &build_context(question, notes_md, summary_md, transcript_md),
        )?
        .trim()
        .to_string();

    if answer.is_empty() {
        bail!("The local model returned an empty answer. Trying again usually fixes it.");
    }
    Ok(answer)
}

/// Assembles the single user turn. Split out so the assembly — including the
/// truncation, which is the part that can silently lose meaning — is testable
/// without a server.
fn build_context(question: &str, notes_md: &str, summary_md: &str, transcript_md: &str) -> String {
    let mut out = String::new();

    if crate::notes::has_content(notes_md) {
        out.push_str("=== The user's own notes from this meeting ===\n");
        out.push_str(notes_md.trim());
        out.push_str("\n\n");
    }
    if !summary_md.trim().is_empty() {
        out.push_str("=== Summary ===\n");
        out.push_str(summary_md.trim());
        out.push_str("\n\n");
    }

    let (transcript, truncated) = trim_front(transcript_md.trim(), MAX_TRANSCRIPT_CHARS);
    if !transcript.is_empty() {
        out.push_str("=== Transcript ===\n");
        if truncated {
            out.push_str(
                "(The earlier part of this transcript was too long to include. \
                 If the question is about the beginning of the recording, say so \
                 rather than answering from what remains.)\n",
            );
        }
        out.push_str(transcript);
        out.push_str("\n\n");
    }

    out.push_str("=== Question ===\n");
    out.push_str(question.trim());
    out
}

/// Keeps the last `max` characters, returning whether anything was dropped.
///
/// Trimming from the front rather than the back is a deliberate choice: the end
/// of a meeting holds the decisions and the action items, which is what people
/// ask about. Cuts land on a line boundary so a sentence is never sliced in
/// half and read as a complete statement.
fn trim_front(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    // Byte index `max` from the end, moved forward to a char boundary and then
    // to the next line start.
    let mut cut = text.len() - max;
    while cut < text.len() && !text.is_char_boundary(cut) {
        cut += 1;
    }
    let tail = &text[cut..];
    let tail = match tail.find('\n') {
        Some(i) => &tail[i + 1..],
        None => tail,
    };
    (tail, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    const TRANSCRIPT: &str = "[00:00:00] **George:** What's the number?\n\
                              [00:00:05] **Speaker 1:** Fifteen percent.\n";
    const SUMMARY: &str = "## TL;DR\nAgreed 15%.";
    const NOTES: &str = "- pricing\n- 15%?";

    fn mock_llm(server: &MockServer) -> LlmClient {
        LlmClient {
            base_url: server.base_url(),
            model: "qwen3:8b".into(),
        }
    }

    #[test]
    fn ask_sends_the_recording_and_returns_the_answer() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/api/chat")
                .body_includes("Fifteen percent")
                .body_includes("what did we agree");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "  You agreed on 15%.  " }
            }));
        });

        let out = ask(
            &mock_llm(&server),
            "what did we agree on pricing",
            NOTES,
            SUMMARY,
            TRANSCRIPT,
        )
        .unwrap();

        assert_eq!(out, "You agreed on 15%.", "the answer should be trimmed");
        m.assert();
    }

    /// The prompt is the only thing standing between this feature and confident
    /// invention, so its instruction is pinned.
    #[test]
    fn the_prompt_forbids_guessing_and_outside_knowledge() {
        let lower = SYSTEM_PROMPT.to_lowercase();
        assert!(lower.contains("never guess"), "{SYSTEM_PROMPT}");
        assert!(lower.contains("outside knowledge"), "{SYSTEM_PROMPT}");
        assert!(
            lower.contains("doesn't come up in this recording"),
            "the model needs an exact phrase to fall back on: {SYSTEM_PROMPT}"
        );
    }

    #[test]
    fn all_three_sources_reach_the_model_notes_first() {
        let ctx = build_context("q", NOTES, SUMMARY, TRANSCRIPT);
        let notes_at = ctx.find("own notes").expect("notes missing");
        let summary_at = ctx.find("=== Summary ===").expect("summary missing");
        let transcript_at = ctx.find("=== Transcript ===").expect("transcript missing");
        assert!(notes_at < summary_at && summary_at < transcript_at, "{ctx}");
        assert!(ctx.contains("15%?"));
        assert!(ctx.contains("Fifteen percent"));
        assert!(
            ctx.trim_end().ends_with('q'),
            "the question goes last: {ctx}"
        );
    }

    #[test]
    fn a_recording_with_no_notes_omits_the_notes_section_entirely() {
        let ctx = build_context("q", "   \n ", SUMMARY, TRANSCRIPT);
        assert!(
            !ctx.contains("own notes"),
            "an empty notes header is noise in the prompt: {ctx}"
        );
    }

    #[test]
    fn an_empty_question_is_refused_before_calling_the_model() {
        // base_url is unroutable: reaching the network at all would hang/err,
        // so a clean message proves we returned before trying.
        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".into(),
            model: "x".into(),
        };
        let err = ask(&llm, "   ", NOTES, SUMMARY, TRANSCRIPT)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "Type a question first.");
    }

    /// Asking about a recording that has not been processed must explain that,
    /// not produce an answer invented from nothing.
    #[test]
    fn asking_about_an_unprocessed_recording_says_so() {
        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".into(),
            model: "x".into(),
        };
        let err = ask(&llm, "what happened", "", "", "")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("hasn't been transcribed yet"),
            "message is not readable by a non-engineer: {err}"
        );
    }

    #[test]
    fn an_empty_model_answer_is_an_error_the_user_can_act_on() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant", "content": "   " }
            }));
        });
        let err = ask(&mock_llm(&server), "q", NOTES, SUMMARY, TRANSCRIPT)
            .unwrap_err()
            .to_string();
        assert!(err.contains("again"), "no suggested next step: {err}");
    }

    // --- truncation ------------------------------------------------------

    #[test]
    fn a_short_transcript_is_not_truncated() {
        let (out, truncated) = trim_front(TRANSCRIPT, MAX_TRANSCRIPT_CHARS);
        assert_eq!(out, TRANSCRIPT);
        assert!(!truncated);
    }

    /// The end of a meeting holds the decisions, so that is the half kept.
    #[test]
    fn an_over_long_transcript_keeps_the_end_and_says_it_was_cut() {
        let mut long = String::new();
        for i in 0..5000 {
            long.push_str(&format!("[00:00:{:02}] **A:** line {i}\n", i % 60));
        }
        long.push_str("[01:00:00] **A:** the decision is fifteen percent\n");

        let (out, truncated) = trim_front(&long, 1000);
        assert!(truncated);
        assert!(out.len() <= 1000);
        assert!(
            out.contains("the decision is fifteen percent"),
            "the end of the meeting was dropped"
        );
        assert!(!out.contains("line 0\n"), "the front should be gone");
    }

    /// A cut that landed mid-sentence would be read as a complete statement.
    #[test]
    fn a_cut_lands_on_a_line_boundary() {
        let text = "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\ndddddddddd\n";
        let (out, truncated) = trim_front(text, 25);
        assert!(truncated);
        assert!(
            text.lines().any(|l| out.starts_with(l)),
            "cut mid-line: {out:?}"
        );
    }

    /// Truncation must never panic on a multi-byte boundary. A bilingual
    /// transcript is the normal case here, not an edge case.
    #[test]
    fn truncating_multibyte_text_does_not_panic_or_corrupt() {
        let mut long = String::new();
        for i in 0..2000 {
            long.push_str(&format!("[00:00:00] **甲:** 我们开始吧 {i}\n"));
        }
        let (out, truncated) = trim_front(&long, 1000);
        assert!(truncated);
        // The proof it landed on a char boundary is that this is valid UTF-8
        // at all, plus that the tail is intact.
        assert!(out.contains("我们开始吧 1999"), "{out}");
    }

    #[test]
    fn the_truncation_notice_reaches_the_model() {
        let long = "x".repeat(MAX_TRANSCRIPT_CHARS + 10);
        let ctx = build_context("q", "", "", &long);
        assert!(
            ctx.contains("too long to include"),
            "the model was not told the transcript was cut"
        );
    }
}
