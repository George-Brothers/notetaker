import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import type { RecordingDetail as RecordingDetailData } from "../lib/ipc";
import { StatusChip } from "./StatusChip";

export interface RecordingDetailProps {
  detail: RecordingDetailData | null;
  loading: boolean;
  onRenameSpeaker: (id: string, key: string, name: string) => void;
  onSaveSummary: (id: string, summaryMd: string) => void;
}

interface TranscriptLine {
  time: string;
  speaker: string;
  text: string;
}

// Matches the Rust side's `to_transcript_md` output:
// "[HH:MM:SS] **Name:** text"
const LINE_PATTERN = /^\[(\d{2}:\d{2}:\d{2})\]\s*\*\*(.+?):\*\*\s?(.*)$/;

function parseTranscript(md: string): TranscriptLine[] {
  return md
    .split("\n")
    .map((line) => line.match(LINE_PATTERN))
    .filter((m): m is RegExpMatchArray => m !== null)
    .map((m) => ({ time: m[1], speaker: m[2], text: m[3] }));
}

function formatDetailDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

export function RecordingDetail({ detail, loading, onRenameSpeaker, onSaveSummary }: RecordingDetailProps) {
  const [summaryDraft, setSummaryDraft] = useState("");
  const [renaming, setRenaming] = useState<{ lineIndex: number; key: string; original: string } | null>(
    null
  );
  const [renameDraft, setRenameDraft] = useState("");

  useEffect(() => {
    setSummaryDraft(detail?.summaryMd ?? "");
    setRenaming(null);
  }, [detail?.id, detail?.summaryMd]);

  if (loading) {
    return (
      <section className="detail-pane detail-pane--empty" aria-live="polite">
        Loading recording…
      </section>
    );
  }

  if (!detail) {
    return (
      <section className="detail-pane detail-pane--empty">
        <p>Select a recording to see its summary and transcript.</p>
      </section>
    );
  }

  const lines = parseTranscript(detail.transcriptMd);
  const nameToKey = Object.entries(detail.speakers).reduce<Record<string, string>>((acc, [key, name]) => {
    acc[name] = key;
    return acc;
  }, {});

  function beginRename(lineIndex: number, speakerName: string) {
    const key = nameToKey[speakerName] ?? speakerName;
    setRenaming({ lineIndex, key, original: speakerName });
    setRenameDraft(speakerName);
  }

  function submitRename(e: FormEvent) {
    e.preventDefault();
    if (!detail || !renaming) return;
    const trimmed = renameDraft.trim();
    if (trimmed && trimmed !== renaming.original) {
      onRenameSpeaker(detail.id, renaming.key, trimmed);
    }
    setRenaming(null);
  }

  return (
    <section className="detail-pane" aria-label="Recording detail">
      <header className="detail-pane__header">
        <h2>{detail.title}</h2>
        <div className="detail-pane__meta">
          <span>{formatDetailDate(detail.created)}</span>
          <span>{detail.task ?? "Unsorted"}</span>
          <StatusChip status={detail.status} error={detail.error} />
        </div>
      </header>

      <section className="detail-pane__section">
        <h3>Summary</h3>
        <p className="detail-pane__hint">
          Edits save automatically when you click away.
        </p>
        <textarea
          className="summary-editor"
          aria-label="Summary"
          value={summaryDraft}
          onChange={(e) => setSummaryDraft(e.target.value)}
          onBlur={() => {
            if (detail && summaryDraft !== detail.summaryMd) {
              onSaveSummary(detail.id, summaryDraft);
            }
          }}
          rows={8}
        />
      </section>

      <section className="detail-pane__section">
        <h3>Transcript</h3>
        {lines.length === 0 ? (
          <p className="empty-state">No transcript yet.</p>
        ) : (
          <div className="transcript" role="log" aria-label="Transcript">
            {lines.map((line, i) => (
              <div className="transcript-line" key={i}>
                <span className="transcript-line__time" title="Playback isn't wired up yet">
                  {line.time}
                </span>
                <span className="transcript-line__speaker">
                  <button type="button" className="speaker-name" onClick={() => beginRename(i, line.speaker)}>
                    {line.speaker}
                  </button>
                  {renaming?.lineIndex === i && (
                    <form className="speaker-rename" onSubmit={submitRename}>
                      <label htmlFor="speaker-rename-input" className="sr-only">
                        Rename {renaming.original}
                      </label>
                      <input
                        id="speaker-rename-input"
                        autoFocus
                        value={renameDraft}
                        onChange={(e) => setRenameDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Escape") setRenaming(null);
                        }}
                      />
                      <button type="submit">Save</button>
                      <button type="button" onClick={() => setRenaming(null)}>
                        Cancel
                      </button>
                    </form>
                  )}
                </span>
                <span className="transcript-line__text">{line.text}</span>
              </div>
            ))}
          </div>
        )}
      </section>
    </section>
  );
}
