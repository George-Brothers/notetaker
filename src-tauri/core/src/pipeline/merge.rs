//! Merge stage: turn raw diarization spans and raw transcribed text into a
//! speaker-labeled transcript. Pure logic — no models, no I/O, no async.

use crate::pipeline::diarize::SpeakerSpan;
use crate::pipeline::Utterance;

/// Assign each text span to the diarization span it overlaps most (interval
/// intersection). If a text span overlaps nothing, fall back to the nearest
/// span. Diarizer speaker `N` (0-based) becomes display label `"Speaker
/// N+1"`.
pub fn label_speakers(spans: &[SpeakerSpan], texts: &[(f32, f32, String)]) -> Vec<Utterance> {
    texts
        .iter()
        .map(|(start_s, end_s, text)| {
            let best = spans
                .iter()
                .map(|span| {
                    let overlap = (span.end_s.min(*end_s) - span.start_s.max(*start_s)).max(0.0);
                    (span, overlap)
                })
                .max_by(|(span_a, overlap_a), (span_b, overlap_b)| {
                    if *overlap_a != *overlap_b {
                        overlap_a.total_cmp(overlap_b)
                    } else {
                        // Tie-break (including the "no overlap" case, where
                        // every overlap is 0.0): fall back to nearest span.
                        // We want the smaller distance to win the max_by,
                        // so compare distances in reverse.
                        let dist_a = distance(span_a, *start_s, *end_s);
                        let dist_b = distance(span_b, *start_s, *end_s);
                        dist_b.total_cmp(&dist_a)
                    }
                })
                .map(|(span, _)| span);

            let speaker = match best {
                Some(span) => format!("Speaker {}", span.speaker + 1),
                None => "Speaker 1".to_string(),
            };

            Utterance {
                start_s: *start_s,
                end_s: *end_s,
                speaker,
                text: text.clone(),
            }
        })
        .collect()
}

/// Distance from a text span `[start_s, end_s]` to a diarization span: 0 if
/// they overlap, otherwise the gap between the nearest edges.
fn distance(span: &SpeakerSpan, start_s: f32, end_s: f32) -> f32 {
    if end_s < span.start_s {
        span.start_s - end_s
    } else if start_s > span.end_s {
        start_s - span.end_s
    } else {
        0.0
    }
}

/// Combine a mic track (already labeled "George") with other speakers'
/// utterances, stable-sorted by start time.
pub fn merge_meeting(mic: Vec<Utterance>, others: Vec<Utterance>) -> Vec<Utterance> {
    let mut merged = mic;
    merged.extend(others);
    merged.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    merged
}

/// Render utterances as markdown lines: `[HH:MM:SS] **Name:** text`.
pub fn to_transcript_md(title: &str, utts: &[Utterance]) -> String {
    let mut out = format!("# {title}\n\n");
    for utt in utts {
        let total_seconds = utt.start_s.max(0.0) as u64;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        out.push_str(&format!(
            "[{:02}:{:02}:{:02}] **{}:** {}\n",
            hours, minutes, seconds, utt.speaker, utt.text
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(speaker: &str, start_s: f32, end_s: f32, text: &str) -> Utterance {
        Utterance {
            start_s,
            end_s,
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    fn sp(speaker: u32, start_s: f32, end_s: f32) -> SpeakerSpan {
        SpeakerSpan {
            start_s,
            end_s,
            speaker,
        }
    }

    #[test]
    fn meeting_merge_interleaves_by_time() {
        let mic = vec![u("George", 5.0, 8.0, "I agree")];
        let others = vec![
            u("Speaker 1", 0.0, 4.0, "大家好"),
            u("Speaker 2", 9.0, 12.0, "Next item"),
        ];
        let m = merge_meeting(mic, others);
        assert_eq!(
            m.iter().map(|x| x.speaker.as_str()).collect::<Vec<_>>(),
            ["Speaker 1", "George", "Speaker 2"]
        );
    }

    #[test]
    fn transcript_md_formats_timestamps() {
        let md = to_transcript_md("T", &[u("George", 3661.5, 3665.0, "hi")]);
        assert!(md.contains("[01:01:01] **George:** hi"), "{md}");
    }

    #[test]
    fn label_speakers_assigns_span_majority_overlap() {
        let spans = vec![sp(0, 0.0, 5.0), sp(1, 5.0, 10.0)];
        let texts = vec![(0.5, 4.5, "hello".into()), (5.5, 9.0, "你好".into())];
        let out = label_speakers(&spans, &texts);
        assert_eq!(out[0].speaker, "Speaker 1");
        assert_eq!(out[1].speaker, "Speaker 2");
    }

    #[test]
    fn label_speakers_falls_back_to_nearest_when_no_overlap() {
        let spans = vec![sp(0, 0.0, 2.0), sp(1, 10.0, 12.0)];
        // Text span [3.0, 4.0] overlaps neither span; nearest is span 0
        // (gap 1.0) vs span 1 (gap 6.0).
        let texts = vec![(3.0, 4.0, "closer to first".into())];
        let out = label_speakers(&spans, &texts);
        assert_eq!(out[0].speaker, "Speaker 1");

        // Text span [8.0, 9.0] overlaps neither span; nearest is span 1
        // (gap 1.0) vs span 0 (gap 6.0).
        let texts2 = vec![(8.0, 9.0, "closer to second".into())];
        let out2 = label_speakers(&spans, &texts2);
        assert_eq!(out2[0].speaker, "Speaker 2");
    }

    #[test]
    fn label_speakers_empty_inputs_yield_empty_output() {
        assert_eq!(label_speakers(&[], &[]), Vec::<Utterance>::new());

        let spans = vec![sp(0, 0.0, 5.0)];
        assert_eq!(label_speakers(&spans, &[]), Vec::<Utterance>::new());

        // No spans at all: falls back to "Speaker 1" for any text.
        let texts = vec![(0.0, 1.0, "alone".into())];
        let out = label_speakers(&[], &texts);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].speaker, "Speaker 1");
    }
}
