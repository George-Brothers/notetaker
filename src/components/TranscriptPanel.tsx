/**
 * The transcript, and its half of the audio.
 *
 * Lifted from Otter and Fathom: click any line to hear it, and the line
 * under the playhead highlights as it goes. Speakers get a colour lane
 * assigned by first appearance, so the colours stay put between visits.
 *
 * The player itself used to live here, on the reasoning that the list and
 * the transport were one feature. They are not: everything below the
 * no-segments guard was unreachable for a recording that had not been
 * processed yet, which meant raw audio could not be played at all. The
 * player is `NoteView`'s now and arrives as a prop, so there is exactly
 * one of it per note.
 *
 * A recording whose transcript has no parseable timestamps (an unprocessed
 * one, or one the user rewrote as prose) yields no segments; the raw
 * markdown is rendered instead. That is a deliberate fallback rather than
 * an empty panel.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type { RecordingDetail } from "../lib/ipc";
import type { AudioPlayer } from "../hooks/useAudio";
import { speakerColor, duration } from "../lib/format";
import { cn } from "../lib/cn";
import { Markdown } from "./Markdown";
import { Chip } from "./ui";

export function TranscriptPanel({
  detail,
  audio,
  onRenameSpeaker,
}: {
  detail: RecordingDetail;
  audio: AudioPlayer;
  /** Takes the diarizer key ("spk1"), not the display name. */
  onRenameSpeaker: (key: string, name: string) => void;
}) {
  const [renaming, setRenaming] = useState<{ key: string; original: string } | null>(null);
  const [draft, setDraft] = useState("");

  const listRef = useRef<HTMLOListElement | null>(null);

  const speakers = useMemo(() => {
    const seen: string[] = [];
    for (const s of detail.segments) {
      if (s.speaker && !seen.includes(s.speaker)) seen.push(s.speaker);
    }
    return seen;
  }, [detail.segments]);

  const activeIndex = useMemo(() => {
    if (!audio.playing && audio.currentTime === 0) return -1;
    return detail.segments.findIndex(
      (s) => audio.currentTime >= s.startS && audio.currentTime < s.endS,
    );
  }, [detail.segments, audio.currentTime, audio.playing]);

  // Keep the highlighted line on screen, but only while playing — scrolling the
  // list out from under someone who is reading it is worse than losing the
  // highlight.
  useEffect(() => {
    if (!audio.playing || activeIndex < 0) return;
    const el = listRef.current?.children[activeIndex] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [activeIndex, audio.playing]);

  if (detail.segments.length === 0) {
    return detail.transcriptMd.trim() ? (
      <Markdown>{detail.transcriptMd}</Markdown>
    ) : (
      <p className="py-8 text-center text-[13px] text-fg-muted">
        {detail.status === "ready"
          ? "This recording has no transcript."
          : "The transcript appears here once this recording has been processed."}
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {speakers.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[11px] text-fg-faint">
            {renaming ? "Type a name:" : "Click a speaker to name them:"}
          </span>
          {speakers.map((name, i) =>
            renaming?.original === name ? (
              <form
                key={name}
                onSubmit={(e) => {
                  e.preventDefault();
                  const next = draft.trim();
                  if (next && next !== renaming.original) onRenameSpeaker(renaming.key, next);
                  setRenaming(null);
                }}
                className="flex items-center gap-1"
              >
                <label className="sr-only" htmlFor="speaker-rename-input">
                  Rename {renaming.original}
                </label>
                <input
                  id="speaker-rename-input"
                  autoFocus
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setRenaming(null);
                  }}
                  className="h-6 w-28 rounded-full border border-accent bg-raised px-2 text-[12px] text-fg focus:outline-none"
                />
                <button
                  type="submit"
                  className="rounded-full px-1.5 text-[12px] font-medium text-accent hover:bg-hover"
                >
                  Save
                </button>
              </form>
            ) : (
              <button
                key={name}
                type="button"
                onClick={() => {
                  // The diarizer key is what `rename_speaker` addresses; the
                  // transcript only carries the display name, so map back
                  // through `speakers`. A name with no key (the mic track's
                  // "George") is its own key, which is what core expects.
                  const key =
                    Object.entries(detail.speakers).find(([, label]) => label === name)?.[0] ??
                    name;
                  setRenaming({ key, original: name });
                  setDraft(name);
                }}
                className="rounded-full transition-opacity hover:opacity-70"
              >
                <Chip color={speakerColor(i)}>{name}</Chip>
              </button>
            ),
          )}
        </div>
      )}

      <ol ref={listRef} className="flex flex-col">
        {detail.segments.map((seg, i) => {
          const lane = seg.speaker ? speakers.indexOf(seg.speaker) : -1;
          const active = i === activeIndex;
          return (
            <li key={`${seg.line}-${seg.startS}`}>
              <button
                type="button"
                onClick={() => audio.seekAndPlay(seg.startS)}
                disabled={!audio.src}
                className={cn(
                  "flex w-full gap-3 rounded-[var(--radius-control)] px-2 py-1.5 text-left transition-colors",
                  active ? "bg-accent-soft" : "hover:bg-hover",
                  !audio.src && "cursor-default",
                )}
              >
                <span className="w-12 shrink-0 pt-0.5 text-right font-mono text-[11px] tabular-nums text-fg-faint">
                  {duration(seg.startS)}
                </span>
                <span className="min-w-0 flex-1">
                  {seg.speaker && (
                    <span
                      className="mr-2 text-[13px] font-semibold"
                      style={{ color: lane >= 0 ? speakerColor(lane) : undefined }}
                    >
                      {seg.speaker}
                    </span>
                  )}
                  <span className={cn("text-[15px] leading-[1.65]", active ? "text-fg" : "text-fg")}>
                    {seg.text}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
