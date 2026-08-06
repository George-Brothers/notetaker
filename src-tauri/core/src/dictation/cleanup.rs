//! Deterministic and local-model cleanup for one dictation utterance.
//!
//! Layer 0 is deliberately independent of Ollama. It removes recognizer
//! artifacts and handles spoken editing commands even when no local model is
//! installed. Layer 1 is an optional, short Ollama call; its prompt is part of
//! the contract because an answer containing commentary is worse than the
//! slightly rough transcript we started with.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use regex::Regex;

use crate::pipeline::llm::{KeepAlive, LlmClient};

/// The exact system contract sent to the local cleanup model.
pub const CLEANUP_SYSTEM_PROMPT: &str = r#"You clean up one dictated utterance.
Return only the cleaned text. Do not add a preface, explanation, markdown, bullets, quotes, or commentary.
Preserve the speaker's meaning, names, numbers, and uncertainty.
Remove filler words and false starts, repair punctuation, and apply an explicit self-correction such as "at 2 actually 3" as "at 3".
Never invent facts or summarize. Keep the original language."#;

fn marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?ix)\[\s*(?:BLANK[_ ]?AUDIO|MUSIC|APPLAUSE|LAUGHTER|NOISE|SILENCE|INAUDIBLE)[^\]]*\]",
        )
        .expect("dictation marker regex")
    })
}

fn new_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:new\s+line|line\s+break)\b").unwrap())
}

/// Applies the no-network cleanup layer. It is safe to call for every
/// utterance, including a one-word answer where the LLM layer is skipped.
pub fn layer0(input: &str) -> String {
    let without_markers = marker_re().replace_all(input, " ");
    let with_newlines = new_line_re().replace_all(&without_markers, "\n");
    let without_scratch = remove_scratch_that(&with_newlines);

    let mut lines = Vec::new();
    for line in without_scratch.lines() {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !normalized.is_empty() {
            lines.push(normalized);
        }
    }
    lines.join("\n")
}

/// Applies user-authored spoken replacements after marker/command cleanup.
/// Keys are matched as whole words, case-insensitively, so a dictionary entry
/// such as `Notetaker` cannot rewrite the middle of `Notetakerly`.
pub fn apply_replacements(input: &str, replacements: &BTreeMap<String, String>) -> String {
    replacements.iter().fold(input.to_string(), |text, (from, to)| {
        let from = from.trim();
        if from.is_empty() {
            return text;
        }
        let pattern = format!(r"(?i)\b{}\b", regex::escape(from));
        match Regex::new(&pattern) {
            Ok(re) => re.replace_all(&text, to.as_str()).into_owned(),
            Err(_) => text,
        }
    })
}

/// Removes the last clause/sentence before a spoken `scratch that` command.
/// Repeating the command is handled from left to right, which mirrors how a
/// person corrects themselves in one utterance.
pub fn remove_scratch_that(input: &str) -> String {
    let mut text = input.to_string();
    loop {
        let lower = text.to_ascii_lowercase();
        let Some(position) = lower.find("scratch that") else {
            break;
        };
        let before = text[..position].trim_end();
        let boundary = before
            .char_indices()
            .rev()
            .find(|(_, ch)| matches!(ch, '.' | '!' | '?' | '\n' | ',' | ';' | ':'))
            .map(|(index, ch)| index + ch.len_utf8());
        let kept = boundary.map_or_else(String::new, |index| before[..index].to_string());
        let after = text[position + "scratch that".len()..].trim_start();
        text = if kept.is_empty() {
            after.to_string()
        } else if after.is_empty() {
            kept
        } else {
            format!("{kept} {after}")
        };
    }
    text
}

/// Whether Layer 1 is worth its latency for this utterance.
pub fn should_run_llm(text: &str) -> bool {
    text.split_whitespace().count() >= 8
}

/// Cleans a transcript with the local-only model and rejects an empty answer.
pub fn layer1(llm: &LlmClient, text: &str) -> Result<String> {
    let reply = llm.chat_with_keep_alive(CLEANUP_SYSTEM_PROMPT, text, KeepAlive::Batch)?;
    let cleaned = clean_model_reply(&reply);
    if cleaned.is_empty() {
        bail!("the local cleanup model returned empty text")
    }
    Ok(cleaned)
}

/// Models in the qwen family may emit a private reasoning block despite the
/// system prompt. It is never part of a dictation transcript.
pub fn clean_model_reply(reply: &str) -> String {
    let mut text = reply.trim().to_string();
    if let Some(start) = text.find("<think>") {
        if let Some(end) = text[start + 7..].find("</think>") {
            text.replace_range(start..start + 7 + end + 8, "");
        }
    }
    text = text.trim().to_string();
    if text.starts_with("```") && text.ends_with("```") {
        let fenced = text
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        let mut lines = fenced.lines();
        let first = lines.next().unwrap_or_default().trim();
        text = if matches!(first, "text" | "plain" | "plaintext" | "markdown" | "md") {
            lines.collect::<Vec<_>>().join("\n")
        } else {
            fenced.to_string()
        };
        text = text.trim().to_string();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer0_removes_blank_audio_markers_and_commands() {
        let text = layer0("Hello [BLANK_AUDIO] new line this is fine scratch that corrected");
        assert_eq!(text, "Hello\ncorrected");
    }

    #[test]
    fn layer0_keeps_previous_sentence_when_scratch_that_replaces_last_clause() {
        assert_eq!(
            layer0("The first point is done. the second point scratch that the third point"),
            "The first point is done. the third point"
        );
    }

    #[test]
    fn short_utterances_skip_the_llm() {
        assert!(!should_run_llm("one two three four five six seven"));
        assert!(should_run_llm("one two three four five six seven eight"));
    }

    #[test]
    fn cleanup_prompt_forbids_commentary_and_preserves_meaning() {
        assert!(CLEANUP_SYSTEM_PROMPT.contains("Return only the cleaned text"));
        assert!(CLEANUP_SYSTEM_PROMPT.contains("at 2 actually 3"));
        assert!(CLEANUP_SYSTEM_PROMPT.contains("Never invent facts"));
    }

    #[test]
    fn model_reasoning_and_fences_do_not_reach_the_transcript() {
        assert_eq!(
            clean_model_reply("<think>internal</think>\n```text\nhello\n```"),
            "hello"
        );
    }
}
