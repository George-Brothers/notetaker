import { useState } from "react";
import type { CaptureState, CaptureStatus, Mode } from "../lib/ipc";

export interface RecordBarProps {
  status: CaptureStatus;
  onStart: (mode: Mode, title: string) => void;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
}

/** Below this, the disk-space warning shows. Matches the session's disk guard floor. */
const LOW_DISK_MB = 1024;

const MODE_COPY: Record<Mode, { label: string; hint: string }> = {
  meeting: {
    label: "Meeting",
    hint: "Records the room and the call's audio",
  },
  in_person: {
    label: "In-person",
    hint: "A class or face-to-face meeting — mic only",
  },
};

const MODES: Mode[] = ["meeting", "in_person"];

function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const ss = String(sec).padStart(2, "0");
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
  return `${m}:${ss}`;
}

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

function LevelMeter({ label, level }: { label: string; level: number }) {
  const pct = Math.round(Math.max(0, Math.min(1, level)) * 100);
  return (
    <div className="level-meter">
      <span className="level-meter__label">{label}</span>
      <span
        className="level-meter__track"
        role="progressbar"
        aria-label={`${label} level`}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <span className="level-meter__fill" style={{ width: `${pct}%` }} />
      </span>
    </div>
  );
}

export function RecordBar({ status, onStart, onPause, onResume, onStop }: RecordBarProps) {
  const [mode, setMode] = useState<Mode>("meeting");
  const idle = status.state === "idle";
  const paused = status.state === "paused";
  // The last recording is still being written. Starting another one here would
  // look like the first never happened, so Start waits — but Stop stays live,
  // because a save that failed is retried by pressing it again.
  const finishing = status.state === "finishing";
  const capturing = status.state === "recording" || paused;

  function handleStart() {
    onStart(mode, defaultTitle(mode));
  }

  function handleToggle() {
    if (status.state === "recording") onPause();
    else if (status.state === "paused") onResume();
  }

  return (
    <div className="record-bar">
      <fieldset className="record-bar__modes">
        <legend className="sr-only">Recording mode</legend>
        {MODES.map((m) => (
          <div key={m} className="record-bar__mode">
            <input
              type="radio"
              id={`mode-${m}`}
              name="capture-mode"
              value={m}
              checked={mode === m}
              disabled={!idle}
              onChange={() => setMode(m)}
              aria-describedby={`mode-${m}-hint`}
            />
            <span className="record-bar__mode-copy">
              {/* The label wraps only the short name, not the hint, so the
                  radio's accessible name stays "Meeting" / "In-person"
                  rather than swallowing the whole description. */}
              <label htmlFor={`mode-${m}`} className="record-bar__mode-label">
                {MODE_COPY[m].label}
              </label>
              <span id={`mode-${m}-hint`} className="record-bar__mode-hint">
                {MODE_COPY[m].hint}
              </span>
            </span>
          </div>
        ))}
      </fieldset>

      <div className="record-bar__controls">
        <button type="button" className="record-bar__start" onClick={handleStart} disabled={!idle}>
          Start
        </button>
        <button type="button" onClick={handleToggle} disabled={!capturing}>
          {paused ? "Resume" : "Pause"}
        </button>
        <button type="button" onClick={onStop} disabled={idle}>
          Stop
        </button>
      </div>

      {finishing && (
        <span className="record-bar__finishing" role="status">
          Saving your recording — it will appear in the library in a moment.
        </span>
      )}

      <span className={`record-bar__state record-bar__state--${status.state}`}>
        <span className="record-bar__state-dot" aria-hidden="true" />
        {stateLabel(status.state)}
      </span>

      <span className="record-bar__timer" aria-label="Elapsed recording time">
        {formatElapsed(status.elapsedS)}
      </span>

      <div className="record-bar__meters">
        <LevelMeter label="Microphone" level={status.micLevel} />
        {mode === "meeting" && <LevelMeter label="System audio" level={status.systemLevel} />}
      </div>

      {status.diskFreeMb < LOW_DISK_MB && (
        <span className="record-bar__disk-warning" role="status">
          Low on disk space — free up some room so this recording isn't cut short.
        </span>
      )}
    </div>
  );
}
