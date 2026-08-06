/**
 * The tray popover: a dumb remote for the main App owner.
 *
 * It never polls capture, reads settings, or starts a second state machine.
 * Rust positions this webview beside the tray icon; App pushes one snapshot
 * over `tray-panel-sync`, and every button emits the same intent events as the
 * native menu.
 */

import { useEffect, useState } from "react";
import {
  ChevronDown,
  ExternalLink,
  FileText,
  Mic,
  Pause,
  Play,
  Settings,
  Square,
  Star,
} from "lucide-react";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { CaptureState, Mode } from "../lib/ipc";
import type { InputDevice } from "../lib/desktop";
import { cn } from "../lib/cn";
import { duration } from "../lib/format";
import { useTheme } from "../hooks/useTheme";

export type TrayModelState = "sleeping" | "loading" | "ready" | "error";

export interface TrayRecentNote {
  id: string;
  title: string;
  created: string;
  durationS: number;
}
export interface TrayPanelSync {
  capture: {
    state: CaptureState;
    mode: Mode | null;
    recordingId: string | null;
    elapsedS: number;
    micLevel: number;
    systemLevel: number;
    appName: string | null;
  };
  recentNotes: TrayRecentNote[];
  inputDevices: InputDevice[];
  selectedInputDevice: string | null;
  modelState: TrayModelState;
  statusLine: string;
}

const EMPTY_SYNC: TrayPanelSync = {
  capture: {
    state: "idle",
    mode: null,
    recordingId: null,
    elapsedS: 0,
    micLevel: 0,
    systemLevel: 0,
    appName: null,
  },
  recentNotes: [],
  inputDevices: [],
  selectedInputDevice: null,
  modelState: "sleeping",
  statusLine: "Not recording",
};

function modelLabel(state: TrayModelState): string {
  switch (state) {
    case "ready":
      return "Models ready";
    case "loading":
      return "Models loading";
    case "error":
      return "Models need attention";
    default:
      return "Models sleeping";
  }
}

function formatCreated(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Recent";
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function meterWidth(value: number): string {
  return `${Math.max(4, Math.min(100, Math.round(value * 100)))}%`;
}

export function TrayPanel() {
  useTheme();
  const [sync, setSync] = useState<TrayPanelSync>(EMPTY_SYNC);
  const [recordMenuOpen, setRecordMenuOpen] = useState(false);

  useEffect(() => {
    const un = listen<TrayPanelSync>("tray-panel-sync", (event) => setSync(event.payload));
    void un.then(() => emit("tray-panel-ready")).catch(() => {});
    un.catch(() => {});
    return () => {
      void un.then((stop) => stop()).catch(() => {});
    };
  }, []);

  const closePanel = () => {
    void getCurrentWindow().hide().catch(() => {});
  };
  const send = (event: string, payload?: unknown) => {
    closePanel();
    void emit(event, payload).catch(() => {});
  };

  const isRecording = sync.capture.state === "recording" || sync.capture.state === "paused";
  const isPaused = sync.capture.state === "paused";
  const isFinishing = sync.capture.state === "finishing";

  const iconButton =
    "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-fg-muted transition-colors hover:bg-hover hover:text-fg focus-visible:outline-accent";
  const secondaryButton =
    "inline-flex items-center justify-center gap-2 rounded-[var(--radius-control)] border border-border bg-raised/60 px-3 py-2 text-[12px] font-medium text-fg transition-colors hover:bg-hover focus-visible:outline-accent";

  return (
    <main className="h-screen w-screen overflow-hidden p-2">
      <div className="floating-glass flex h-full min-h-0 flex-col overflow-hidden p-4 text-fg">
        {isRecording || isFinishing ? (
          <section aria-labelledby="tray-recording-heading" className="flex min-h-0 flex-1 flex-col">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-recording">
                  {isFinishing ? "Finishing" : "Recording"}
                </p>
                <h1 id="tray-recording-heading" className="mt-1 truncate text-[18px] font-semibold tabular-nums">
                  {duration(sync.capture.elapsedS)}
                </h1>
                <p className="mt-1 truncate text-[12px] text-fg-muted">
                  {sync.capture.appName ??
                    (sync.capture.mode === "in_person" ? "In-person recording" : "Meeting recording")}
                </p>
              </div>
              <span className="rounded-full bg-recording-soft px-2 py-1 text-[11px] font-medium text-recording">
                {isPaused ? "Paused" : isFinishing ? "Saving" : "Live"}
              </span>
            </div>

            <div className="mt-5 space-y-3" aria-label="Audio levels">
              <LevelMeter label="Microphone" value={sync.capture.micLevel} color="bg-accent" />
              {sync.capture.mode !== "in_person" && (
                <LevelMeter label="System audio" value={sync.capture.systemLevel} color="bg-accent-2" />
              )}
            </div>

            <p className="mt-5 rounded-[var(--radius-control)] bg-sunken/70 px-3 py-2 text-[12px] text-fg-muted">
              {sync.statusLine}
            </p>

            <div className="mt-auto grid grid-cols-3 gap-2 pt-5">
              <button type="button" className={secondaryButton} disabled={isFinishing} onClick={() => send("tray-pause-resume")}>
                {isPaused ? <Play size={14} /> : <Pause size={14} />}
                {isPaused ? "Resume" : "Pause"}
              </button>
              <button
                type="button"
                className={cn(secondaryButton, "border-recording/40 text-recording hover:bg-recording-soft")}
                disabled={isFinishing}
                onClick={() => send("tray-stop")}
              >
                <Square size={13} />
                Stop
              </button>
              <button type="button" className={secondaryButton} disabled={isFinishing} onClick={() => send("tray-highlight")}>
                <Star size={14} />
                Star
              </button>
            </div>
          </section>
        ) : (
          <section aria-labelledby="tray-idle-heading" className="flex min-h-0 flex-1 flex-col">
            <div className="flex items-center justify-between gap-3">
              <div>
                <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-accent">Notetaker</p>
                <h1 id="tray-idle-heading" className="mt-1 text-[18px] font-semibold">Ready to record</h1>
              </div>
              <Mic size={18} className="text-accent" aria-hidden="true" />
            </div>

            <div className="relative mt-4 flex">
              <button
                type="button"
                className="inline-flex min-h-10 flex-1 items-center justify-center gap-2 rounded-l-[var(--radius-control)] bg-recording px-3 text-[13px] font-semibold text-accent-fg transition-opacity hover:opacity-90 focus-visible:outline-accent"
                onClick={() => send("tray-record", "meeting")}
              >
                <Mic size={15} />
                Record meeting
              </button>
              <button
                type="button"
                aria-label="Choose recording type"
                aria-expanded={recordMenuOpen}
                className="min-h-10 rounded-r-[var(--radius-control)] border-l border-recording-soft bg-recording px-3 text-accent-fg transition-opacity hover:opacity-90 focus-visible:outline-accent"
                onClick={() => setRecordMenuOpen((open) => !open)}
              >
                <ChevronDown size={15} />
              </button>
              {recordMenuOpen && (
                <div className="absolute left-0 top-[calc(100%+0.4rem)] z-10 w-full rounded-[var(--radius-control)] border border-border bg-raised p-1 shadow-pop">
                  <button
                    type="button"
                    className="flex w-full items-center rounded px-2 py-2 text-left text-[12px] text-fg hover:bg-hover"
                    onClick={() => send("tray-record", "meeting")}
                  >
                    Record meeting
                  </button>
                  <button
                    type="button"
                    className="flex w-full items-center rounded px-2 py-2 text-left text-[12px] text-fg hover:bg-hover"
                    onClick={() => send("tray-record", "in_person")}
                  >
                    Record in person
                  </button>
                </div>
              )}
            </div>

            <label className="mt-4 flex items-center gap-2 rounded-[var(--radius-control)] border border-border bg-sunken/60 px-3 py-2 text-[12px] text-fg-muted">
              <Mic size={14} className="shrink-0 text-accent" />
              <span className="shrink-0">Microphone</span>
              <select
                aria-label="Microphone"
                className="min-w-0 flex-1 bg-transparent text-right text-[12px] text-fg outline-none"
                value={sync.selectedInputDevice ?? ""}
                onChange={(event) => {
                  closePanel();
                  void emit("tray-mic-select", event.target.value || null).catch(() => {});
                }}
              >
                <option value="">System default</option>
                {sync.inputDevices.map((device) => (
                  <option key={device.id} value={device.id}>
                    {device.label}{device.isDefault ? " (default)" : ""}
                  </option>
                ))}
              </select>
            </label>

            <div className="mt-4 flex items-center gap-2 text-[12px] text-fg-muted">
              <span className={cn("h-2 w-2 rounded-full", sync.modelState === "ready" ? "bg-ok" : "bg-fg-faint")} />
              {modelLabel(sync.modelState)}
            </div>

            <div className="mt-5 min-h-0 flex-1">
              <div className="flex items-center justify-between">
                <h2 className="text-[12px] font-semibold text-fg">Recent notes</h2>
                <FileText size={14} className="text-fg-faint" aria-hidden="true" />
              </div>
              {sync.recentNotes.length === 0 ? (
                <p className="mt-3 text-[12px] text-fg-faint">Your recent notes will appear here.</p>
              ) : (
                <div className="mt-2 space-y-1 overflow-y-auto">
                  {sync.recentNotes.slice(0, 5).map((note) => (
                    <button
                      type="button"
                      key={note.id}
                      className="flex w-full items-center justify-between gap-3 rounded-[var(--radius-control)] px-2 py-2 text-left transition-colors hover:bg-hover focus-visible:outline-accent"
                      onClick={() => send("tray-open-note", note.id)}
                    >
                      <span className="min-w-0 truncate text-[12px] text-fg">{note.title}</span>
                      <span className="shrink-0 text-[11px] text-fg-faint">{formatCreated(note.created)}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          </section>
        )}

        <footer className="mt-3 flex items-center justify-between border-t border-border/70 pt-3">
          <button type="button" className={cn(iconButton, "gap-1 px-2 text-[12px]")} onClick={() => send("tray-open-settings")}>
            <Settings size={14} />
            Settings
          </button>
          <button type="button" className={cn(iconButton, "w-auto gap-1 px-2 text-[12px]")} onClick={() => send("tray-open")}>
            Open Notetaker
            <ExternalLink size={13} />
          </button>
        </footer>
      </div>
    </main>
  );
}

function LevelMeter({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div>
      <div className="mb-1 flex justify-between text-[11px] text-fg-muted">
        <span>{label}</span>
        <span>{Math.round(Math.max(0, Math.min(1, value)) * 100)}%</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-sunken" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={1} aria-valuenow={value}>
        <div className={cn("h-full rounded-full transition-[width]", color)} style={{ width: meterWidth(value) }} />
      </div>
    </div>
  );
}
