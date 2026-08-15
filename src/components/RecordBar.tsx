/**
 * The record control, as a pill rather than a bar.
 *
 * It used to be a full-width strip carrying two radio buttons, three buttons,
 * two meters and a timer — about a fifth of the window, permanently, for
 * something that is idle almost all of the time. Now it is one control that
 * grows into the recording state and shrinks back afterwards.
 *
 * What did *not* change is which controls are live when. `finishing` is not
 * idle: the last recording is still being written, and starting another one
 * there would look like the first never happened. Stop stays live throughout,
 * because a save that failed is retried by pressing it again.
 */

import { useState } from "react";
import { ChevronDown, Circle, Mic, Pause, Play, Square, Users } from "lucide-react";
import type { CaptureState, CaptureStatus, Mode } from "../lib/ipc";
import { duration } from "../lib/format";
import { cn } from "../lib/cn";
import { Button, IconButton, Popover, PopoverContent, PopoverTrigger, Tip } from "./ui";

export interface RecordBarProps {
  status: CaptureStatus;
  onStart: (mode: Mode, title: string) => void;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
}

/** Below this, the disk-space warning shows. Matches the session's disk guard floor. */
const LOW_DISK_MB = 1024;

const MODE_COPY: Record<Mode, { label: string; hint: string; icon: React.ReactNode }> = {
  meeting: {
    label: "Meeting",
    hint: "Records the room and the call's audio",
    icon: <Users size={14} />,
  },
  in_person: {
    label: "In person",
    hint: "A class or face-to-face meeting — mic only",
    icon: <Mic size={14} />,
  },
};

const MODES: Mode[] = ["meeting", "in_person"];

function defaultTitle(mode: Mode): string {
  const stamp = new Date().toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  return `${MODE_COPY[mode].label} ${stamp}`;
}

function stateLabel(state: CaptureState): string {
  switch (state) {
    case "recording":
      return "Recording";
    case "paused":
      return "Paused";
    case "finishing":
      return "Saving…";
    case "idle":
      return "Idle";
  }
}

/**
 * A live level meter.
 *
 * Kept as a `progressbar` with a real `aria-valuenow` rather than a decorative
 * div: "is it actually hearing me" is the question this answers, and it is the
 * one thing on screen a person cannot check any other way.
 */
function LevelMeter({
  label,
  short,
  level,
}: {
  /** The accessible name. Stays spelled out — "Mic level" is not a name. */
  label: string;
  /** What the pill has room to print. */
  short: string;
  level: number;
}) {
  const pct = Math.round(Math.max(0, Math.min(1, level)) * 100);
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-[11px] text-fg-faint">{short}</span>
      <span
        role="progressbar"
        aria-label={`${label} level`}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        className="h-1 w-14 overflow-hidden rounded-full bg-sunken"
      >
        <span
          className="block h-full rounded-full bg-[image:var(--grad-aurora)] shadow-[var(--glow-accent)] transition-[width] duration-50 ease-linear"
          style={{ width: `${pct}%` }}
        />
      </span>
    </div>
  );
}

export function RecordBar({ status, onStart, onPause, onResume, onStop }: RecordBarProps) {
  const [mode, setMode] = useState<Mode>("meeting");
  const [menuOpen, setMenuOpen] = useState(false);
  const idle = status.state === "idle";
  const paused = status.state === "paused";
  const finishing = status.state === "finishing";
  const capturing = status.state === "recording" || paused;

  function start(m: Mode) {
    setMode(m);
    setMenuOpen(false);
    onStart(m, defaultTitle(m));
  }

  if (idle || finishing) {
    return (
      <div className="record-bar record-bar--idle flex flex-wrap items-center gap-2">
        <div className="record-bar__main">
          <Button variant="primary" size="md" onClick={() => start(mode)} disabled={!idle}>
            <Circle size={13} fill="currentColor" />
            Record
          </Button>

          <Popover open={menuOpen} onOpenChange={setMenuOpen}>
            <PopoverTrigger asChild>
              <Button
                variant="secondary"
                size="icon"
                disabled={!idle}
                aria-label="Choose what to record"
              >
                <ChevronDown size={14} />
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-64">
              <fieldset>
                <legend className="sr-only">Recording mode</legend>
                {MODES.map((m) => (
                  <button
                    key={m}
                    type="button"
                    onClick={() => start(m)}
                    className="flex w-full items-start gap-2.5 rounded-[var(--radius-control)] px-2 py-2 text-left transition-colors hover:bg-hover"
                  >
                    <span className="mt-0.5 shrink-0 text-fg-faint">{MODE_COPY[m].icon}</span>
                    <span className="flex flex-col">
                      <span className="text-[13px] font-medium text-fg">{MODE_COPY[m].label}</span>
                      <span className="text-[12px] leading-snug text-fg-muted">
                        {MODE_COPY[m].hint}
                      </span>
                    </span>
                  </button>
                ))}
              </fieldset>
            </PopoverContent>
          </Popover>

          {finishing && (
            <span role="status" className="record-bar__status text-[12px] text-fg-muted">
              Saving your recording — it appears in the library in a moment.
            </span>
          )}
        </div>

        {/* Shown while idle too, not only mid-recording: the useful moment to
            learn the disk is nearly full is before a two-hour lecture, not
            ninety minutes into one. */}
        {status.diskFreeMb < LOW_DISK_MB && (
          <span role="status" className="record-bar__warning text-[12px]">
            Low on disk space — free up room so this recording isn't cut short.
          </span>
        )}
      </div>
    );
  }

  return (
    <div
      className={cn(
        "record-bar record-bar--live flex flex-wrap items-center gap-3 rounded-full border px-2 py-1",
        paused ? "border-border bg-sunken" : "border-recording/30 bg-recording-soft",
      )}
    >
      <div className="record-bar__main">
        <span className="flex items-center gap-1.5 pl-1">
          <span
            aria-hidden
            className={cn(
              "h-2 w-2 rounded-full",
              paused ? "bg-fg-faint" : "animate-pulse bg-recording",
            )}
          />
          <span
            className={cn("text-[12px] font-medium", paused ? "text-fg-muted" : "text-recording")}
          >
            {stateLabel(status.state)}
          </span>
        </span>

        <span
          aria-label="Elapsed recording time"
          className="font-mono text-[13px] tabular-nums text-fg"
        >
          {duration(status.elapsedS)}
        </span>

        <span className="flex items-center gap-0.5">
          <IconButton
            label={paused ? "Resume" : "Pause"}
            onClick={paused ? onResume : onPause}
            disabled={!capturing}
          >
            {paused ? <Play size={14} /> : <Pause size={14} />}
          </IconButton>
          <Tip label="Stop and save">
            <Button variant="danger" size="sm" onClick={onStop} className="rounded-full">
              <Square size={11} fill="currentColor" />
              Stop
            </Button>
          </Tip>
        </span>
      </div>

      {/* Meters only while a session is live. A meter pinned at zero because
          nothing is being captured reads as "your microphone is dead", which is
          the one thing it must never say by accident. `status.mode` drives the
          system meter rather than the picker's local state: what matters is
          what the running session is actually capturing. */}
      <div className="record-bar__meters">
        <LevelMeter label="Microphone" short="Mic" level={status.micLevel} />
        {status.mode === "meeting" && (
          <LevelMeter label="System audio" short="Call" level={status.systemLevel} />
        )}
      </div>

      {status.diskFreeMb < LOW_DISK_MB && (
        <span role="status" className="record-bar__warning text-[12px]">
          Low on disk space — free up room so this recording isn't cut short.
        </span>
      )}
    </div>
  );
}
