/**
 * The transcript, and the audio that goes with it.
 *
 * Lifted from Otter and Fathom: click any line to hear it, and the line under
 * the playhead highlights as it goes. Speakers get a colour lane assigned by
 * first appearance, so the colours stay put between visits.
 *
 * The player and the list live in one component because they are one feature —
 * splitting them would mean lifting `currentTime` into a parent that has no
 * other use for it, and re-rendering the whole note four times a second.
 *
 * A recording whose transcript has no parseable timestamps (an unprocessed one,
 * or one the user rewrote as prose) yields no segments; the raw markdown is
 * rendered instead. That is a deliberate fallback rather than an empty panel.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pause, Play, SkipBack, Volume2 } from "lucide-react";
import type { RecordingDetail } from "../lib/ipc";
import { audioSrc } from "../lib/transport";
import { duration, speakerColor } from "../lib/format";
import { cn } from "../lib/cn";
import { Markdown } from "./Markdown";
import { Button, Chip, IconButton, Notice } from "./ui";

/** Human labels for the two tracks. */
const TRACK_LABEL: Record<string, string> = {
  mic: "Your microphone",
  system: "Everyone else",
};

function useAudio(recordingId: string, track: string | null) {
  const ref = useRef<HTMLAudioElement | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    setError(null);
    setCurrentTime(0);
    setPlaying(false);
    if (!track) return;
    audioSrc(recordingId, track)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [recordingId, track]);

  const seek = useCallback((seconds: number) => {
    const el = ref.current;
    if (!el) return;
    el.currentTime = Math.max(0, seconds);
    setCurrentTime(el.currentTime);
    void el.play().catch(() => {
      // Autoplay policy can refuse a play() the user did not initiate. The
      // seek still landed; they can press play.
    });
  }, []);

  const toggle = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    if (el.paused) void el.play().catch(() => setPlaying(false));
    else el.pause();
  }, []);

  return { ref, src, error, playing, setPlaying, currentTime, setCurrentTime, seek, toggle };
}

export function TranscriptPanel({
  detail,
  onRenameSpeaker,
}: {
  detail: RecordingDetail;
  /** Takes the diarizer key ("spk1"), not the display name. */
  onRenameSpeaker: (key: string, name: string) => void;
}) {
  const tracks = detail.audioTracks;
  const [track, setTrack] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ key: string; original: string } | null>(null);
  const [draft, setDraft] = useState("");

  // Default to the track that has the other people on it — that is what you
  // replay. Re-picked whenever the recording changes.
  useEffect(() => {
    setTrack(tracks.includes("system") ? "system" : (tracks[0] ?? null));
  }, [detail.id, tracks]);

  const audio = useAudio(detail.id, track);
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
      {tracks.length > 0 && (
        <div className="sticky top-0 z-10 -mx-1 flex flex-col gap-2 rounded-[var(--radius-card)] border border-border bg-raised/95 px-3 py-2 backdrop-blur">
          <div className="flex items-center gap-2">
            <Button
              variant="primary"
              size="icon"
              onClick={audio.toggle}
              disabled={!audio.src}
              aria-label={audio.playing ? "Pause" : "Play"}
            >
              {audio.playing ? <Pause size={15} /> : <Play size={15} />}
            </Button>
            <IconButton
              label="Back to the start"
              onClick={() => audio.seek(0)}
              disabled={!audio.src}
            >
              <SkipBack size={15} />
            </IconButton>

            <input
              type="range"
              min={0}
              max={Math.max(1, detail.durationS)}
              step={0.5}
              value={Math.min(audio.currentTime, detail.durationS)}
              onChange={(e) => audio.seek(Number(e.target.value))}
              aria-label="Position in the recording"
              className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-sunken accent-[var(--c-accent)]"
            />
            <span className="shrink-0 font-mono text-[12px] tabular-nums text-fg-muted">
              {duration(audio.currentTime)} / {duration(detail.durationS)}
            </span>
          </div>

          {tracks.length > 1 && (
            <div className="flex items-center gap-1.5">
              <Volume2 size={13} className="text-fg-faint" aria-hidden />
              {tracks.map((t) => (
                <button
                  key={t}
                  type="button"
                  onClick={() => setTrack(t)}
                  aria-pressed={track === t}
                  className={cn(
                    "rounded-full px-2 py-0.5 text-[12px] transition-colors",
                    track === t
                      ? "bg-accent-soft font-medium text-accent"
                      : "text-fg-muted hover:bg-hover",
                  )}
                >
                  {TRACK_LABEL[t] ?? t}
                </button>
              ))}
            </div>
          )}

          {audio.error && <Notice tone="warn">{audio.error}</Notice>}

          {audio.src && (
            <audio
              ref={audio.ref}
              src={audio.src}
              preload="metadata"
              onPlay={() => audio.setPlaying(true)}
              onPause={() => audio.setPlaying(false)}
              onEnded={() => audio.setPlaying(false)}
              onTimeUpdate={(e) => audio.setCurrentTime(e.currentTarget.currentTime)}
              onError={() =>
                audio.setPlaying(false)
              }
              className="hidden"
            />
          )}
        </div>
      )}

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
                onClick={() => audio.seek(seg.startS)}
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
