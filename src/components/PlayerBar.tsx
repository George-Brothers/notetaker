/**
 * The thin player under a recording's title.
 *
 * Opened from the `Listen` toggle in the note's toolbar, and minimised back
 * into it — minimised, not closed: audio keeps running so it can play while
 * notes are being typed, and the toggle stays lit so the sound is never a
 * mystery.
 *
 * It renders one of three things: the transport, "still recording", or "no
 * audio". The last two are sentences rather than disabled controls, because a
 * greyed-out button explains nothing.
 */

import type { KeyboardEvent } from "react";
import {
  ChevronDown,
  Gauge,
  Pause,
  Play,
  RotateCcw,
  RotateCw,
  SkipBack,
  Volume2,
} from "lucide-react";
import type { AudioPlayer } from "../hooks/useAudio";
import { RATES, SKIP_SECONDS } from "../hooks/useAudio";
import { duration } from "../lib/format";
import { cn } from "../lib/cn";
import {
  Button,
  IconButton,
  Notice,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "./ui";

/** Human labels for the two tracks. */
export const TRACK_LABEL: Record<string, string> = {
  mic: "Your microphone",
  system: "Everyone else",
};

export interface PlayerBarProps {
  audio: AudioPlayer;
  durationS: number;
  tracks: string[];
  track: string | null;
  onTrackChange: (track: string) => void;
  onCollapse: () => void;
  /** True when this recording is the one currently capturing. */
  live: boolean;
}

const SHELL =
  "player-bar mt-3 flex flex-wrap items-center gap-2 rounded-[var(--radius-card)] border border-border bg-raised px-3 py-2";

function Minimise({ onCollapse }: { onCollapse: () => void }) {
  return (
    <IconButton label="Minimise the player" onClick={onCollapse} className="shrink-0">
      <ChevronDown size={15} />
    </IconButton>
  );
}

export function PlayerBar({
  audio,
  durationS,
  tracks,
  track,
  onTrackChange,
  onCollapse,
  live,
}: PlayerBarProps) {
  if (live) {
    return (
      <div className={SHELL} role="group" aria-label="Playback">
        <span className="text-[13px] text-fg-muted">
          This recording is still going. It'll be listenable as soon as you stop.
        </span>
        <Minimise onCollapse={onCollapse} />
      </div>
    );
  }

  if (tracks.length === 0) {
    return (
      <div className={SHELL} role="group" aria-label="Playback">
        <span className="text-[13px] text-fg-muted">No audio was saved for this recording.</span>
        <Minimise onCollapse={onCollapse} />
      </div>
    );
  }

  // Space and the arrows belong to the player only while the player has
  // focus, and never when the event came from something typable — the
  // notepad shares this screen. The scrubber is an input, so the arrow keys
  // stay its own too.
  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    const target = e.target as HTMLElement;
    if (
      target.tagName === "INPUT" ||
      target.tagName === "TEXTAREA" ||
      target.isContentEditable
    ) {
      return;
    }
    if (e.key === " " || e.key === "Spacebar") {
      e.preventDefault();
      audio.toggle();
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      audio.skip(-SKIP_SECONDS);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      audio.skip(SKIP_SECONDS);
    }
  }

  return (
    <div className={SHELL} role="group" aria-label="Playback" onKeyDown={onKeyDown}>
      <div className="player-bar__transport">
        <IconButton label="Back to the start" onClick={() => audio.seek(0)}>
          <SkipBack size={15} />
        </IconButton>
        <IconButton label={`Back ${SKIP_SECONDS} seconds`} onClick={() => audio.skip(-SKIP_SECONDS)}>
          <RotateCcw size={15} />
        </IconButton>
        <Button
          variant="primary"
          size="icon"
          onClick={audio.toggle}
          aria-label={audio.playing ? "Pause" : "Play"}
        >
          {audio.playing ? <Pause size={15} /> : <Play size={15} />}
        </Button>
        <IconButton
          label={`Forward ${SKIP_SECONDS} seconds`}
          onClick={() => audio.skip(SKIP_SECONDS)}
        >
          <RotateCw size={15} />
        </IconButton>
      </div>

      <div className="player-bar__timeline">
        <input
          type="range"
          min={0}
          max={Math.max(1, durationS)}
          step={0.5}
          value={Math.min(audio.currentTime, Math.max(1, durationS))}
          onChange={(e) => audio.seek(Number(e.target.value))}
          aria-label="Position in the recording"
          className="h-1 cursor-pointer appearance-none rounded-full bg-sunken accent-[var(--c-accent)]"
        />

        <span className="shrink-0 font-mono text-[12px] tabular-nums text-fg-muted">
          {duration(audio.currentTime)} / {duration(durationS)}
        </span>
      </div>

      <div className="player-bar__options">
        <Popover>
          <PopoverTrigger asChild>
            <Button size="sm" variant="ghost" aria-label="Playback speed">
              <Gauge size={13} />
              {audio.rate}×
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-32">
            {RATES.map((r) => (
              <button
                key={r}
                type="button"
                onClick={() => audio.setRate(r)}
                className={cn(
                  "w-full rounded-[var(--radius-control)] px-2 py-1.5 text-left text-[13px] hover:bg-hover",
                  r === audio.rate ? "bg-selected font-medium text-fg" : "text-fg-muted",
                )}
              >
                {r}×
              </button>
            ))}
          </PopoverContent>
        </Popover>

        {tracks.length > 1 ? (
          <Popover>
            <PopoverTrigger asChild>
              <Button size="sm" variant="ghost" aria-label="Which track you are hearing" className="max-w-36 min-w-0">
                <Volume2 size={13} />
                <span className="truncate">{TRACK_LABEL[track ?? ""] ?? track}</span>
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-48">
              {tracks.map((t) => (
                <button
                  key={t}
                  type="button"
                  onClick={() => onTrackChange(t)}
                  className={cn(
                    "w-full rounded-[var(--radius-control)] px-2 py-1.5 text-left text-[13px] hover:bg-hover",
                    t === track ? "bg-selected font-medium text-fg" : "text-fg-muted",
                  )}
                >
                  {TRACK_LABEL[t] ?? t}
                </button>
              ))}
            </PopoverContent>
          </Popover>
        ) : (
          <span className="flex max-w-36 min-w-0 shrink items-center gap-1 text-[12px] text-fg-muted">
            <Volume2 size={13} className="shrink-0 text-fg-faint" aria-hidden />
            <span className="truncate">{TRACK_LABEL[track ?? ""] ?? track}</span>
          </span>
        )}

        <Minimise onCollapse={onCollapse} />
      </div>

      {audio.error && (
        <Notice tone="warn" className="w-full">
          {audio.error}
        </Notice>
      )}
    </div>
  );
}
