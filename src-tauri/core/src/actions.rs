//! Action items, read out of and written back into `summary.md`.
//!
//! There is deliberately **no separate storage** for the checklist. The
//! summarizer already emits `- [ ] Alice: send the deck` inside the summary,
//! `summary.md` is already user-editable, and `api::update_summary` already
//! persists edits to it. A second file holding "which boxes are ticked" would
//! immediately disagree with the markdown the moment the user edited either
//! one, and there would be no principled way to say which was right.
//!
//! So a tick *is* an edit to that one line of markdown, and the markdown is
//! the single source of truth. The cost is that ticking a box rewrites
//! `summary.md`; the benefit is that the checklist survives the user editing
//! their notes by hand, and a recording copied to another machine carries its
//! ticks with it.
//!
//! # Which lines count
//!
//! Every GitHub-style checkbox line in the document, in document order —
//! *not* only those under an `## Action items` heading. The heading is written
//! by a language model and comes back as "Action items", "Action Items",
//! "Next steps" or occasionally nothing at all, so anchoring to it would drop
//! real items unpredictably. Anchoring to the checkbox syntax instead means a
//! box the user typed themselves under any heading is also an action item,
//! which is the behaviour a user would expect anyway.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// One checkbox line from `summary.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    /// Position among the checkbox lines, in document order, 0-based. This is
    /// what [`set_done`] takes. It is *not* stable across an edit that adds or
    /// removes an item — which is fine, because the UI re-reads the list after
    /// every write.
    pub index: usize,
    /// The line's text with the `- [ ] ` marker stripped, owner included, as
    /// written. What to show if you only show one thing.
    pub text: String,
    /// The `Name:` prefix, if the line has one. Extracted for display as a
    /// separate chip; `text` still contains it so nothing is lost when the
    /// extraction guesses wrong.
    pub owner: Option<String>,
    pub done: bool,
    /// 0-based line number in `summary.md`, so the UI can scroll the summary
    /// to the item the user just ticked.
    pub line: usize,
}

/// Every checkbox line in `summary_md`, in document order.
pub fn parse(summary_md: &str) -> Vec<ActionItem> {
    let mut out = Vec::new();
    for (line_no, line) in summary_md.lines().enumerate() {
        if let Some((done, body)) = split_checkbox(line) {
            out.push(ActionItem {
                index: out.len(),
                owner: owner_of(body),
                text: body.trim().to_string(),
                done,
                line: line_no,
            });
        }
    }
    out
}

/// Returns `summary_md` with checkbox number `index` set to `done`.
///
/// Only the one marker changes: the line's text, its indentation, its bullet
/// character and every other line in the file are preserved byte for byte.
/// That matters because the user's own prose lives in this file, and a
/// checkbox toggle that reformatted their summary would be a data-loss bug
/// wearing a feature's clothes.
pub fn set_done(summary_md: &str, index: usize, done: bool) -> Result<String> {
    let mut seen = 0usize;
    let mut found = false;
    let mut out = String::with_capacity(summary_md.len());

    for line in summary_md.split_inclusive('\n') {
        // Split the trailing newline off so the marker search never sees it.
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => (b.strip_suffix('\r').unwrap_or(b), newline_of(line)),
            None => (line, ""),
        };

        if split_checkbox(body).is_some() {
            if seen == index {
                out.push_str(&rewrite_marker(body, done));
                found = true;
            } else {
                out.push_str(body);
            }
            seen += 1;
        } else {
            out.push_str(body);
        }
        out.push_str(eol);
    }

    if !found {
        bail!("this recording has no action item number {}", index + 1);
    }
    Ok(out)
}

/// The exact line ending a line used, so CRLF files stay CRLF. A summary
/// edited in Notepad on Windows comes back with CRLF, and rewriting it to LF
/// would show the whole file as changed.
fn newline_of(line: &str) -> &'static str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Splits a checkbox line into `(done, text-after-the-marker)`, or `None` if
/// the line is not one.
///
/// Accepts `-`, `*` and `+` bullets, any leading indentation, and `x` or `X`
/// for ticked — all four appear in real model output and in markdown people
/// type. Requires the space after the marker, so a literal `- [x]hello`
/// (which no renderer treats as a checkbox) is not one here either.
fn split_checkbox(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))?;
    let rest = rest.trim_start();

    for (marker, done) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(after) = rest.strip_prefix(marker) {
            // A checkbox with nothing after it is a marker, not an item.
            if after.is_empty() {
                return None;
            }
            if let Some(text) = after.strip_prefix(' ') {
                return Some((done, text));
            }
            if let Some(text) = after.strip_prefix('\t') {
                return Some((done, text));
            }
            return None;
        }
    }
    None
}

/// Rewrites the `[ ]`/`[x]` marker in a line already known to be a checkbox,
/// leaving everything else untouched.
fn rewrite_marker(line: &str, done: bool) -> String {
    let wanted = if done { "[x]" } else { "[ ]" };
    // Find the first `[` that begins a marker. `split_checkbox` has already
    // confirmed one exists, so the search cannot miss.
    for (i, _) in line.char_indices() {
        let tail = &line[i..];
        if tail.starts_with("[ ]") || tail.starts_with("[x]") || tail.starts_with("[X]") {
            return format!("{}{}{}", &line[..i], wanted, &line[i + 3..]);
        }
    }
    line.to_string()
}

/// Pulls a `Name:` prefix off an item, if it has one that looks like a person
/// rather than a sentence.
///
/// Deliberately conservative. The summarizer is *asked* for `Owner: task`, but
/// it also writes things like "Note: this depends on legal" and
/// "Decision: ship Friday", and showing "Note" as a person's name in an owner
/// chip is worse than showing no owner at all. So: must be short, must have no
/// sentence punctuation, and must not be one of the words models reach for as
/// a label.
fn owner_of(text: &str) -> Option<String> {
    let (candidate, _) = text.split_once(':')?;
    let candidate = candidate.trim();

    if candidate.is_empty() || candidate.chars().count() > 24 {
        return None;
    }
    // A name is one to three words.
    let words = candidate.split_whitespace().count();
    if words == 0 || words > 3 {
        return None;
    }
    // Sentence punctuation means we split mid-sentence, not after a name.
    if candidate.contains(['.', ',', ';', '!', '?', '(', ')', '/']) {
        return None;
    }
    // Must start like a name.
    if !candidate.starts_with(|c: char| c.is_uppercase()) {
        return None;
    }
    const NOT_NAMES: &[&str] = &[
        "note",
        "notes",
        "decision",
        "decisions",
        "action",
        "actions",
        "todo",
        "to do",
        "next",
        "next steps",
        "follow up",
        "reminder",
        "deadline",
        "due",
        "owner",
        "open question",
        "blocker",
        "risk",
        "warning",
        "important",
        "status",
        "update",
    ];
    if NOT_NAMES.contains(&candidate.to_lowercase().as_str()) {
        return None;
    }

    Some(candidate.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMMARY: &str = "\
## TL;DR
We are shipping Friday.

## Action items
- [ ] Alice: send the deck
- [x] Bob: book the room
- [ ] follow up with legal

## Open questions
- Not a checkbox, just a bullet.
";

    // --- parsing ---------------------------------------------------------

    #[test]
    fn parses_every_checkbox_in_document_order() {
        let items = parse(SUMMARY);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].text, "Alice: send the deck");
        assert_eq!(items[0].index, 0);
        assert!(!items[0].done);
        assert_eq!(items[1].text, "Bob: book the room");
        assert!(items[1].done);
        assert_eq!(items[2].text, "follow up with legal");
    }

    #[test]
    fn a_plain_bullet_is_not_an_action_item() {
        let items = parse(SUMMARY);
        assert!(
            !items.iter().any(|i| i.text.contains("just a bullet")),
            "a plain bullet was treated as a checkbox"
        );
    }

    #[test]
    fn line_numbers_point_at_the_right_line() {
        let items = parse(SUMMARY);
        let lines: Vec<&str> = SUMMARY.lines().collect();
        for item in &items {
            assert!(
                lines[item.line].contains(&item.text),
                "line {} is {:?}, expected to contain {:?}",
                item.line,
                lines[item.line],
                item.text
            );
        }
    }

    #[test]
    fn accepts_the_bullet_and_marker_spellings_that_actually_occur() {
        let md = "- [ ] dash\n* [x] star\n+ [ ] plus\n  - [X] indented capital X\n";
        let items = parse(md);
        assert_eq!(items.len(), 4, "{items:?}");
        assert_eq!(
            items.iter().map(|i| i.done).collect::<Vec<_>>(),
            [false, true, false, true]
        );
    }

    #[test]
    fn a_bare_marker_with_no_text_is_not_an_item() {
        // The model sometimes emits an empty checkbox when it has nothing to
        // say. An empty row in the checklist looks like a bug.
        assert!(parse("- [ ]\n- [x]\n").is_empty());
    }

    #[test]
    fn a_marker_without_a_space_after_it_is_not_a_checkbox() {
        // No markdown renderer treats this as one either.
        assert!(parse("- [x]nospace\n").is_empty());
    }

    #[test]
    fn empty_and_checkbox_free_summaries_yield_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("## TL;DR\nJust prose.\n").is_empty());
    }

    // --- owner extraction ------------------------------------------------

    #[test]
    fn an_owner_prefix_is_extracted_but_left_in_the_text() {
        let items = parse("- [ ] Alice: send the deck\n");
        assert_eq!(items[0].owner.as_deref(), Some("Alice"));
        assert_eq!(
            items[0].text, "Alice: send the deck",
            "the text must stay intact so nothing is lost if the guess is wrong"
        );
    }

    #[test]
    fn a_two_word_name_is_an_owner() {
        let items = parse("- [ ] Dr. Rivera: approve the budget\n");
        // "Dr. Rivera" contains a period, so it is deliberately not claimed
        // as an owner rather than risking a wrong chip.
        assert_eq!(items[0].owner, None);

        let items = parse("- [ ] Jordan Lee: approve the budget\n");
        assert_eq!(items[0].owner.as_deref(), Some("Jordan Lee"));
    }

    #[test]
    fn an_item_with_no_colon_has_no_owner() {
        let items = parse("- [ ] follow up with legal\n");
        assert_eq!(items[0].owner, None);
    }

    /// The failure this prevents: an owner chip reading "Note", because the
    /// model wrote "Note: this depends on legal".
    #[test]
    fn a_label_the_model_reaches_for_is_not_treated_as_a_person() {
        for line in [
            "- [ ] Note: this depends on legal\n",
            "- [ ] Decision: ship Friday\n",
            "- [ ] TODO: chase the invoice\n",
            "- [ ] Deadline: Friday 5pm\n",
            "- [ ] Next steps: draft the memo\n",
        ] {
            let items = parse(line);
            assert_eq!(items[0].owner, None, "wrongly claimed an owner in {line:?}");
        }
    }

    #[test]
    fn a_sentence_before_a_colon_is_not_an_owner() {
        let items = parse("- [ ] Ask the vendor, then confirm: pricing for Q4\n");
        assert_eq!(items[0].owner, None);
    }

    #[test]
    fn a_lowercase_word_is_not_an_owner() {
        let items = parse("- [ ] pricing: confirm with the vendor\n");
        assert_eq!(items[0].owner, None);
    }

    // --- toggling --------------------------------------------------------

    #[test]
    fn ticking_an_item_changes_only_that_marker() {
        let out = set_done(SUMMARY, 0, true).unwrap();
        assert!(out.contains("- [x] Alice: send the deck"));
        // Everything else byte-identical.
        assert_eq!(
            out.replace("- [x] Alice", "- [ ] Alice"),
            SUMMARY,
            "toggling rewrote something other than the one marker"
        );
    }

    #[test]
    fn unticking_an_item_works_too() {
        let out = set_done(SUMMARY, 1, false).unwrap();
        assert!(out.contains("- [ ] Bob: book the room"));
        assert!(!parse(&out)[1].done);
    }

    #[test]
    fn setting_an_already_correct_state_is_a_no_op() {
        assert_eq!(set_done(SUMMARY, 1, true).unwrap(), SUMMARY);
        assert_eq!(set_done(SUMMARY, 0, false).unwrap(), SUMMARY);
    }

    /// The user's own prose lives in this file. A toggle that reflowed it
    /// would be data loss dressed as a feature.
    #[test]
    fn toggling_preserves_indentation_bullet_and_surrounding_text() {
        let md = "prose above\n  + [ ] indented plus item\nprose below\n";
        let out = set_done(md, 0, true).unwrap();
        assert_eq!(
            out,
            "prose above\n  + [x] indented plus item\nprose below\n"
        );
    }

    #[test]
    fn crlf_line_endings_survive_a_toggle() {
        let md = "## Action items\r\n- [ ] Alice: send it\r\n";
        let out = set_done(md, 0, true).unwrap();
        assert_eq!(out, "## Action items\r\n- [x] Alice: send it\r\n");
    }

    #[test]
    fn a_file_with_no_trailing_newline_keeps_not_having_one() {
        let md = "- [ ] last line, no newline";
        let out = set_done(md, 0, true).unwrap();
        assert_eq!(out, "- [x] last line, no newline");
    }

    #[test]
    fn a_capital_x_is_normalized_when_toggled() {
        let out = set_done("- [X] shouty\n", 0, true).unwrap();
        assert_eq!(out, "- [x] shouty\n");
    }

    /// Square brackets inside the item text must not be mistaken for the
    /// marker when rewriting.
    #[test]
    fn brackets_in_the_item_text_are_not_mistaken_for_the_marker() {
        let md = "- [ ] Alice: check [the appendix] and [x] on page 3\n";
        let out = set_done(md, 0, true).unwrap();
        assert_eq!(
            out, "- [x] Alice: check [the appendix] and [x] on page 3\n",
            "rewrote a bracket in the text instead of the marker"
        );
    }

    #[test]
    fn an_index_past_the_end_is_an_error_a_user_could_read() {
        let err = set_done(SUMMARY, 9, true).unwrap_err().to_string();
        assert!(
            err.contains("action item"),
            "message is not plain English: {err}"
        );
        // 1-based in the message, because "number 0" means nothing to a user.
        assert!(err.contains("10"), "message should be 1-based: {err}");
    }

    #[test]
    fn toggling_in_a_summary_with_no_checkboxes_errors_rather_than_corrupting() {
        let md = "## TL;DR\nNo actions here.\n";
        assert!(set_done(md, 0, true).is_err());
        assert_eq!(md, "## TL;DR\nNo actions here.\n");
    }

    /// Round-trip: parse, toggle every item, re-parse, and the states must all
    /// have flipped and nothing else moved.
    #[test]
    fn toggling_every_item_flips_exactly_those_states() {
        let before = parse(SUMMARY);
        let mut md = SUMMARY.to_string();
        for item in &before {
            md = set_done(&md, item.index, !item.done).unwrap();
        }
        let after = parse(&md);
        assert_eq!(after.len(), before.len());
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(a.text, b.text, "text changed");
            assert_eq!(a.line, b.line, "line moved");
            assert_ne!(a.done, b.done, "state did not flip for {:?}", b.text);
        }
    }
}
