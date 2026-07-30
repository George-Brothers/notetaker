//! Reading `transcript.md` back into timed segments, so the UI can follow
//! along with the audio and jump to a line.
//!
//! The pipeline writes `[HH:MM:SS] **Name:** text` lines
//! ([`pipeline::merge::to_transcript_md`](crate::pipeline::merge::to_transcript_md)),
//! which is both what a human reads and — because the timestamp is right
//! there — enough to drive a player. So there is no `transcript.json` sidecar:
//! parsing the markdown means every recording ever made already has clickable
//! timestamps, including ones processed before this feature existed.
//!
//! The parse is strict and lossy on purpose. A line that does not match is
//! skipped rather than guessed at, and a file where *nothing* matches yields
//! an empty list — the UI treats that as "no segments" and falls back to
//! rendering the raw markdown, which is exactly right for a transcript a user
//! has hand-edited into prose.

use serde::{Deserialize, Serialize};

/// One spoken stretch, ready to seek to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    /// Seconds from the start of the recording. What the player seeks to.
    pub start_s: f64,
    /// Where this segment stops being the current one: the next segment's
    /// start, or `total_duration_s` for the last. Derived, not stored — the
    /// markdown only carries start times.
    pub end_s: f64,
    pub speaker: String,
    pub text: String,
    /// 0-based line in `transcript.md`. Lets a rename or a jump address the
    /// exact line without re-deriving it in TypeScript.
    pub line: usize,
}

/// Parses `transcript.md` into segments.
///
/// `total_duration_s` closes the last segment; pass the recording's duration.
/// A zero or nonsensical duration is tolerated — the last segment then ends
/// where it starts, which the UI renders as a zero-length highlight rather
/// than a segment that never ends.
pub fn parse(transcript_md: &str, total_duration_s: f64) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();

    for (line_no, line) in transcript_md.lines().enumerate() {
        let Some((start_s, rest)) = split_timestamp(line) else {
            continue;
        };
        let (speaker, text) = split_speaker(rest);
        out.push(Segment {
            start_s,
            // Provisional; fixed up below once the next start is known.
            end_s: start_s,
            speaker,
            text: text.to_string(),
            line: line_no,
        });
    }

    // Close each segment at the next one's start. Done as a second pass
    // because a single pass would need the next line before writing this one.
    for i in 0..out.len() {
        let next_start = out.get(i + 1).map(|s| s.start_s);
        let end = next_start.unwrap_or(total_duration_s);
        // Never end before starting, whatever the inputs claim. A transcript
        // whose timestamps go backwards (or a duration of 0 with segments in
        // it) must not produce a negative-length highlight.
        out[i].end_s = end.max(out[i].start_s);
    }

    out
}

/// Splits a leading `[HH:MM:SS] ` off a line, returning the time in seconds
/// and the remainder.
///
/// Also accepts `[MM:SS]`, because a hand-edited transcript often loses the
/// hour field and dropping those lines would silently shorten the transcript.
fn split_timestamp(line: &str) -> Option<(f64, &str)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (stamp, after) = rest.split_once(']')?;

    let mut seconds = 0f64;
    let mut parts = 0;
    for part in stamp.split(':') {
        let value: u64 = part.trim().parse().ok()?;
        seconds = seconds * 60.0 + value as f64;
        parts += 1;
    }
    if !(2..=3).contains(&parts) {
        return None;
    }

    Some((seconds, after.trim_start()))
}

/// Splits `**Name:** text` into its two halves.
///
/// A line with no speaker tag keeps its whole text and an empty speaker, which
/// the UI renders unattributed rather than inventing a name.
fn split_speaker(rest: &str) -> (String, &str) {
    if let Some(after_open) = rest.strip_prefix("**") {
        if let Some((label, text)) = after_open.split_once("**") {
            let speaker = label.trim().trim_end_matches(':').trim();
            return (speaker.to_string(), text.trim_start());
        }
    }
    (String::new(), rest)
}

/// The distinct speaker names in a transcript, in first-appearance order.
///
/// Used for the player's speaker lanes and their colour assignment. First
/// appearance rather than alphabetical, so a two-person call always puts the
/// person who opened it in the first lane and the colours stay put between
/// visits.
pub fn speakers(segments: &[Segment]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in segments {
        if !s.speaker.is_empty() && !out.contains(&s.speaker) {
            out.push(s.speaker.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = "\
# Accounting 302 — midterm review

[00:00:00] **George:** Let's start with the balance sheet.
[00:00:12] **Speaker 1:** 大家好，我们开始吧。
[00:01:05] **George:** Good point.
";

    #[test]
    fn parses_speaker_time_and_text() {
        let segs = parse(TRANSCRIPT, 90.0);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].start_s, 0.0);
        assert_eq!(segs[0].speaker, "George");
        assert_eq!(segs[0].text, "Let's start with the balance sheet.");
        assert_eq!(segs[1].start_s, 12.0);
        assert_eq!(segs[1].speaker, "Speaker 1");
        assert_eq!(segs[2].start_s, 65.0);
    }

    #[test]
    fn the_title_line_is_not_a_segment() {
        let segs = parse(TRANSCRIPT, 90.0);
        assert!(
            !segs.iter().any(|s| s.text.contains("midterm review")),
            "the markdown title was parsed as speech"
        );
    }

    #[test]
    fn cjk_text_survives_intact() {
        let segs = parse(TRANSCRIPT, 90.0);
        assert_eq!(segs[1].text, "大家好，我们开始吧。");
    }

    /// Each segment must end where the next begins, or the highlight that
    /// follows the audio would either flicker off or overlap two lines.
    #[test]
    fn each_segment_ends_where_the_next_starts() {
        let segs = parse(TRANSCRIPT, 90.0);
        assert_eq!(segs[0].end_s, 12.0);
        assert_eq!(segs[1].end_s, 65.0);
    }

    #[test]
    fn the_last_segment_ends_at_the_recording_duration() {
        let segs = parse(TRANSCRIPT, 90.0);
        assert_eq!(segs.last().unwrap().end_s, 90.0);
    }

    #[test]
    fn hours_are_parsed_as_hours() {
        let segs = parse("[01:01:01] **A:** late\n", 4000.0);
        assert_eq!(segs[0].start_s, 3661.0);
    }

    /// A hand-edited transcript often loses the hour field. Dropping those
    /// lines would silently shorten the transcript, which is worse than
    /// reading `[02:30]` as two and a half minutes.
    #[test]
    fn a_two_field_timestamp_is_read_as_minutes_and_seconds() {
        let segs = parse("[02:30] **A:** hi\n", 200.0);
        assert_eq!(segs[0].start_s, 150.0);
    }

    #[test]
    fn a_line_with_no_timestamp_is_skipped_not_guessed_at() {
        let segs = parse("just prose\n\n**George:** no timestamp\n", 10.0);
        assert!(segs.is_empty());
    }

    /// The whole fallback contract: a transcript the user has rewritten as
    /// prose yields nothing here, and the UI shows the raw markdown instead of
    /// an empty panel.
    #[test]
    fn a_transcript_with_no_parseable_lines_yields_no_segments() {
        assert!(parse("", 10.0).is_empty());
        assert!(parse("# Title\n\nSome prose I typed myself.\n", 10.0).is_empty());
        assert!(parse("[not a time] **A:** hi\n", 10.0).is_empty());
    }

    #[test]
    fn a_timestamped_line_without_a_speaker_tag_keeps_its_text() {
        let segs = parse("[00:00:05] no speaker here\n", 10.0);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].speaker, "");
        assert_eq!(segs[0].text, "no speaker here");
    }

    #[test]
    fn line_numbers_address_the_source_line() {
        let segs = parse(TRANSCRIPT, 90.0);
        let lines: Vec<&str> = TRANSCRIPT.lines().collect();
        for s in &segs {
            assert!(
                lines[s.line].contains(&s.text),
                "line {} is {:?}",
                s.line,
                lines[s.line]
            );
        }
    }

    /// A duration of 0 with segments in it is nonsense input, and it used to
    /// be the shape that produced a negative-length final highlight.
    #[test]
    fn a_zero_duration_never_produces_a_backwards_segment() {
        let segs = parse("[00:00:30] **A:** hi\n", 0.0);
        assert_eq!(segs[0].start_s, 30.0);
        assert_eq!(segs[0].end_s, 30.0);
        for s in &segs {
            assert!(s.end_s >= s.start_s, "backwards segment: {s:?}");
        }
    }

    #[test]
    fn timestamps_that_go_backwards_never_produce_a_backwards_segment() {
        let segs = parse("[00:00:30] **A:** later\n[00:00:10] **B:** earlier\n", 60.0);
        for s in &segs {
            assert!(s.end_s >= s.start_s, "backwards segment: {s:?}");
        }
    }

    // --- speaker lanes ---------------------------------------------------

    #[test]
    fn speakers_are_listed_once_in_first_appearance_order() {
        let segs = parse(TRANSCRIPT, 90.0);
        assert_eq!(speakers(&segs), vec!["George", "Speaker 1"]);
    }

    #[test]
    fn unattributed_segments_contribute_no_speaker() {
        let segs = parse("[00:00:01] no tag\n[00:00:02] **A:** tagged\n", 10.0);
        assert_eq!(speakers(&segs), vec!["A"]);
    }

    #[test]
    fn a_transcript_with_no_speakers_has_no_lanes() {
        assert!(speakers(&[]).is_empty());
    }
}
