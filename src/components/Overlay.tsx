/**
 * The floating recording overlay: a dumb remote for the main App owner.
 *
 * The same webview changes size between the pill and the expanded panel. It
 * does not poll capture or write notes itself; it receives `overlay-sync` and
 * emits intent events back to App, just like the tray panel.
 */

import { useEffect, useRef, useState } from "react";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ChevronDown,
  LoaderCircle,
  Mic,
  Pause,
  Play,
  Send,
  Square,
  Star,
  X,
} from "lucide-react";
import { Markdown } from "./Markdown";
import { useTheme } from "../hooks/useTheme";
import { duration } from "../lib/format";
import { api, type DictationState, type LiveTranscriptEvent } from "../lib/ipc";
import { isNearBottom, mergeLiveTranscript } from "../lib/liveTranscript";
import { isDesktop } from "../lib/transport";
import { cn } from "../lib/cn";

/** What the main window tells this pill/panel to show. */
export interface OverlaySync {
  /** "prompt" = meeting detected, nothing recording yet. */
  kind: "recording" | "prompt" | "dictation";
  state: "recording" | "paused" | "finishing";
  elapsedS: number;
  recordingId: string | null;
  micLevel: number;
  systemLevel: number;
  /** The app that owns the active or detected meeting. */
  appName: string | null;
  /** Lines returned by the append-only highlight writer. */
  highlights: string[];
  /** State wording from the main owner, not an invented UI status. */
  statusLine: string;
  /** Settings can choose the token-backed solid fallback for comparison. */
  style?: "glass" | "solid";
  /** New chunked-batch transcript events since the last owner snapshot. */
  liveTranscript?: LiveTranscriptEvent[];
  /** Present only in the system-wide dictation flow. */
  dictationState?: DictationState;
  dictationText?: string;
  dictationMessage?: string | null;
}

const PILL_SIZE = { width: 300, height: 48 };
const EXPANDED_SIZE = { width: 420, height: 560 };

export function Overlay() {
  useTheme();
  const [sync, setSync] = useState<OverlaySync | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [jot, setJot] = useState("");
  const sizeRef = useRef(PILL_SIZE);
  const [transcript, setTranscript] = useState<LiveTranscriptEvent[]>([]);
  const [question, setQuestion] = useState("");
  const [askId, setAskId] = useState<string | null>(null);
  const [answer, setAnswer] = useState("");
  const [askError, setAskError] = useState<string | null>(null);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const nearBottomRef = useRef(true);
  const transcriptRecordingRef = useRef<string | null>(null);

  useEffect(() => {
    const un = listen<OverlaySync>("overlay-sync", (event) => {
      const payload = event.payload;
      const newRecording = payload.recordingId !== transcriptRecordingRef.current;
      transcriptRecordingRef.current = payload.recordingId;
      setSync(payload);
      setTranscript((current) =>
        (payload.liveTranscript ?? []).reduce(
          mergeLiveTranscript,
          newRecording ? [] : current,
        ),
      );
    });
    un.catch(() => {});
    return () => {
      void un.then((stop) => stop()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!isDesktop()) return;
    const target = expanded ? EXPANDED_SIZE : PILL_SIZE;
    const start = sizeRef.current;
    const startedAt = performance.now();
    const animationMs = 200;
    let frame = 0;
    const window = getCurrentWindow();
    const animate = (now: number) => {
      const progress = Math.min(1, (now - startedAt) / animationMs);
      const eased = 1 - Math.pow(1 - progress, 3);
      const width = Math.round(start.width + (target.width - start.width) * eased);
      const height = Math.round(start.height + (target.height - start.height) * eased);
      sizeRef.current = { width, height };
      void window.setSize(new LogicalSize(width, height)).catch(() => {});
      if (progress < 1) frame = requestAnimationFrame(animate);
    };
    frame = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(frame);
  }, [expanded]);

  useEffect(() => {
    const pane = transcriptRef.current;
    if (pane && nearBottomRef.current) pane.scrollTop = pane.scrollHeight;
  }, [transcript]);

  useEffect(() => {
    if (!askId) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const poll = async () => {
      try {
        const events = await api.pollLiveAsk(askId);
        if (cancelled) return;
        let done = false;
        for (const event of events) {
          if (event.token) setAnswer((current) => current + event.token);
          if (event.error) setAskError(event.error);
          if (event.done) done = true;
        }
        if (done) {
          setAskId(null);
        } else {
          timer = setTimeout(poll, 100);
        }
      } catch (error) {
        if (!cancelled) {
          setAskError(error instanceof Error ? error.message : String(error));
          setAskId(null);
        }
      }
    };

    void poll();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [askId]);

  const send = (event: string, payload?: unknown) => {
    void emit(event, payload).catch(() => {});
  };

  // Nothing yet: the native window is hidden until its first sync. This guard
  // also avoids a blank transparent rectangle during startup.
  if (!sync) return null;

  const iconBtn = cn(
    "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full",
    "text-fg-muted transition-colors hover:bg-hover hover:text-fg",
    "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent",
  );
  const actionBtn =
    "inline-flex min-h-9 items-center justify-center gap-2 rounded-[var(--radius-control)] border border-border bg-raised/55 px-3 text-[12px] font-medium text-fg transition-colors hover:bg-hover focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-45";
  const isPaused = sync.state === "paused";
  const isFinishing = sync.state === "finishing";
  const surface = sync.style === "solid" ? "border border-border bg-raised shadow-pop" : "floating-glass";

  if (sync.kind === "dictation") {
    const processing = sync.dictationState !== "recording";
    const level = Math.max(0, Math.min(1, sync.micLevel));
    return (
      <main className="h-screen w-screen p-1">
        <div
          className={cn(
            surface,
            "floating-glass--pill flex h-full items-center gap-2 overflow-hidden px-3 text-fg",
          )}
          aria-label="System-wide dictation"
        >
          <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-recording" />
          <span className="shrink-0 text-[11px] font-semibold uppercase tracking-[0.1em] text-recording">
            {processing ? "Working" : "Dictating"}
          </span>
          <div className="flex min-w-0 flex-1 items-center gap-0.5" aria-label="Dictation level" role="meter" aria-valuemin={0} aria-valuemax={1} aria-valuenow={level}>
            {Array.from({ length: 18 }, (_, index) => {
              const wave = (Math.sin(index * 1.7) + 1) / 2;
              const height = Math.max(3, Math.round(5 + level * (13 + wave * 10)));
              return <span key={index} className="w-1 rounded-full bg-accent transition-[height] duration-100" style={{ height }} />;
            })}
          </div>
          <span className="max-w-28 truncate text-[11px] text-fg-muted">{dictationStatusLabel(sync.dictationState)}</span>
          <button type="button" aria-label="Cancel dictation" className={iconBtn} onClick={() => send("overlay-dictation-cancel")}>
            <X size={13} />
          </button>
          <button type="button" aria-label="Stop dictation" className={cn(iconBtn, "hover:bg-recording-soft hover:text-recording")} onClick={() => send("overlay-dictation-stop")}>
            <Square size={12} />
          </button>
        </div>
      </main>
    );
  }

  if (sync.kind === "prompt") {
    return (
      <main className="h-screen w-screen p-1">
        <div className={cn(surface, "floating-glass--pill flex h-full items-center gap-2 overflow-hidden px-3 text-fg")}>
          <Mic size={14} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1 truncate text-[12px] text-fg">
            Record {sync.appName ?? "this meeting"}?
          </span>
          <button
            type="button"
            onClick={() => send("overlay-record")}
            className="shrink-0 rounded-full bg-recording px-3 py-1 text-[12px] font-medium text-accent-fg transition-opacity hover:opacity-90 focus-visible:outline-accent"
          >
            Record
          </button>
          <button type="button" aria-label="Dismiss" className={iconBtn} onClick={() => send("overlay-dismiss")}>
            <X size={13} />
          </button>
        </div>
      </main>
    );
  }

  return (
    <main className="h-screen w-screen p-1">
      <div
        className={cn(
          surface,
          "flex h-full w-full flex-col overflow-hidden text-fg",
          !expanded && "floating-glass--pill",
        )}
      >
        <header className="flex min-h-12 items-center gap-2 px-3">
          <span
            className={cn(
              "h-2 w-2 shrink-0 rounded-full",
              sync.state === "recording" ? "animate-pulse bg-recording" : "bg-fg-faint",
            )}
          />
          <button
            type="button"
            aria-label={expanded ? "Collapse overlay" : "Expand overlay"}
            className="min-w-0 flex-1 truncate text-left focus-visible:outline-accent"
            onClick={() => setExpanded((value) => !value)}
          >
            <span
              className={cn(
                "block truncate text-[12px] font-semibold tabular-nums",
                sync.state === "recording" ? "text-recording" : "text-fg-muted",
              )}
            >
              {isFinishing ? "Finishing…" : duration(sync.elapsedS)}
              {isPaused && " — paused"}
            </span>
            {expanded && (
              <span className="block truncate text-[11px] text-fg-muted">
                {sync.appName ?? "Current recording"}
              </span>
            )}
          </button>
          {expanded && <span className="max-w-32 truncate text-[11px] text-fg-faint">{sync.statusLine}</span>}
          <button
            type="button"
            aria-label={expanded ? "Collapse overlay" : "Expand overlay"}
            className={iconBtn}
            onClick={() => setExpanded((value) => !value)}
          >
            <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} size={14} />
          </button>
        </header>

        {!expanded ? (
          <div className="flex min-h-0 flex-1 items-center gap-1 px-2 pb-2">
            {!isFinishing && (
              <>
                <button type="button" aria-label="Star this moment" className={iconBtn} onClick={() => send("overlay-highlight")}>
                  <Star size={13} />
                </button>
                <button type="button" aria-label={isPaused ? "Resume" : "Pause"} className={iconBtn} onClick={() => send("overlay-pause-resume")}>
                  {isPaused ? <Play size={13} /> : <Pause size={13} />}
                </button>
                <button type="button" aria-label="Stop recording" className={cn(iconBtn, "hover:bg-recording-soft hover:text-recording")} onClick={() => send("overlay-stop")}>
                  <Square size={12} />
                </button>
              </>
            )}
            <button type="button" aria-label="Open Notetaker" className={iconBtn} onClick={() => send("overlay-open")}>
              <Mic size={13} />
            </button>
          </div>
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
            <div className="space-y-3">
              <LevelMeter label="Microphone" value={sync.micLevel} color="bg-accent" />
              {sync.systemLevel > 0 && <LevelMeter label="System audio" value={sync.systemLevel} color="bg-accent-2" />}
            </div>

            <div className="mt-4 flex items-center justify-between gap-3">
              <span className="truncate text-[12px] text-fg-muted">{sync.appName ?? "Current recording"}</span>
              <span className="shrink-0 rounded-full bg-recording-soft px-2 py-1 text-[11px] text-recording">
                {isPaused ? "Paused" : isFinishing ? "Saving" : "Live"}
              </span>
            </div>

            <section className="mt-5" aria-labelledby="overlay-transcript-heading">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h2 id="overlay-transcript-heading" className="text-[12px] font-semibold">Live transcript</h2>
                  <p className="mt-1 text-[11px] text-fg-faint">Chunked transcription — usually a few seconds behind.</p>
                </div>
                <span className="shrink-0 rounded-full bg-sunken px-2 py-1 text-[10px] text-fg-faint">local</span>
              </div>
              <div
                ref={transcriptRef}
                onScroll={(event) => {
                  nearBottomRef.current = isNearBottom(event.currentTarget);
                }}
                className="mt-2 max-h-44 space-y-2 overflow-y-auto rounded-[var(--radius-control)] bg-sunken/45 p-2"
                aria-live="polite"
              >
                {transcript.length === 0 ? (
                  <p className="py-3 text-center text-[11px] text-fg-faint">Waiting for speech…</p>
                ) : (
                  transcript.map((message, index) => (
                    <div key={`${message.speaker}-${index}`} className="text-[12px] leading-5">
                      <span className="mr-1 font-semibold text-fg-muted">{message.speaker === "me" ? "Me" : "Them"}</span>
                      <span className="text-fg-ai">{message.text}</span>
                      {message.isPartial && !message.isFinal && <span className="ml-1 text-[10px] text-fg-faint">…</span>}
                    </div>
                  ))
                )}
              </div>
            </section>

            <section className="mt-5" aria-labelledby="overlay-ask-heading">
              <h2 id="overlay-ask-heading" className="text-[12px] font-semibold">Ask about this meeting</h2>
              <form
                className="mt-2 flex gap-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (!question.trim() || askId) return;
                  setAnswer("");
                  setAskError(null);
                  void api
                    .startLiveAsk(
                      question,
                      transcript.map((message) => `${message.speaker}: ${message.text}`).join("\n"),
                    )
                    .then(setAskId)
                    .catch((error) => setAskError(error instanceof Error ? error.message : String(error)));
                }}
              >
                <input
                  value={question}
                  onChange={(event) => setQuestion(event.target.value)}
                  placeholder="What have we decided?"
                  aria-label="Ask about this meeting"
                  className="min-w-0 flex-1 rounded-[var(--radius-control)] border border-border bg-sunken/65 px-2 py-2 text-[12px] text-fg placeholder:text-fg-faint focus-visible:outline-accent"
                />
                <button
                  type="submit"
                  aria-label="Ask locally"
                  className={cn(actionBtn, "w-10 px-0")}
                  disabled={!question.trim() || askId !== null}
                >
                  {askId ? <LoaderCircle size={13} className="animate-spin" /> : <Send size={13} />}
                </button>
              </form>
              <p className="mt-1 text-[10px] text-fg-faint">Answers use this rolling transcript and stay on this computer.</p>
              {askError && <p className="mt-2 text-[11px] text-recording">{askError}</p>}
              {answer && (
                <div className="mt-2 max-h-40 overflow-y-auto rounded-[var(--radius-control)] bg-sunken/45 p-2">
                  <Markdown muted className="text-[12px] leading-5">{answer}</Markdown>
                </div>
              )}
            </section>

            <div className="mt-4 grid grid-cols-3 gap-2">
              <button type="button" className={actionBtn} disabled={isFinishing} onClick={() => send("overlay-pause-resume")}>
                {isPaused ? <Play size={13} /> : <Pause size={13} />}
                {isPaused ? "Resume" : "Pause"}
              </button>
              <button type="button" className={cn(actionBtn, "border-recording/40 text-recording hover:bg-recording-soft")} disabled={isFinishing} onClick={() => send("overlay-stop")}>
                <Square size={12} />
                Stop
              </button>
              <button type="button" className={actionBtn} disabled={isFinishing} onClick={() => send("overlay-highlight")}>
                <Star size={13} />
                Star
              </button>
            </div>

            <section className="mt-5" aria-labelledby="overlay-highlights-heading">
              <div className="flex items-center justify-between">
                <h2 id="overlay-highlights-heading" className="text-[12px] font-semibold">Starred moments</h2>
                <span className="text-[11px] text-fg-faint">{sync.highlights.length}</span>
              </div>
              {sync.highlights.length === 0 ? (
                <p className="mt-2 text-[12px] text-fg-faint">Star a moment to keep it handy here.</p>
              ) : (
                <ul className="mt-2 space-y-1">
                  {sync.highlights.map((highlight, index) => (
                    <li key={`${highlight}-${index}`} className="rounded-[var(--radius-control)] bg-sunken/60 px-2 py-2 text-[12px] text-fg-ai">
                      {highlight.replace(/^-\s*/, "")}
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="mt-5" aria-labelledby="overlay-jot-heading">
              <h2 id="overlay-jot-heading" className="text-[12px] font-semibold">Quick text jot</h2>
              <textarea
                value={jot}
                onChange={(event) => setJot(event.target.value)}
                placeholder="Append a note to this recording…"
                rows={3}
                className="mt-2 w-full resize-none rounded-[var(--radius-control)] border border-border bg-sunken/65 p-2 text-[12px] text-fg placeholder:text-fg-faint focus-visible:outline-accent"
              />
              <button
                type="button"
                className={cn(actionBtn, "mt-2 w-full bg-accent text-accent-fg hover:bg-accent-hover")}
                disabled={!jot.trim() || !sync.recordingId || isFinishing}
                onClick={() => {
                  if (!sync.recordingId || !jot.trim()) return;
                  send("overlay-jot", { recordingId: sync.recordingId, text: jot });
                  setJot("");
                }}
              >
                <Send size={13} />
                Append to notes
              </button>
              <p className="mt-2 text-[11px] text-fg-faint">Added to notes.md without replacing existing notes.</p>
            </section>
          </div>
        )}
      </div>
    </main>
  );
}

function LevelMeter({ label, value, color }: { label: string; value: number; color: string }) {
  const safe = Math.max(0, Math.min(1, value));
  return (
    <div>
      <div className="mb-1 flex justify-between text-[11px] text-fg-muted">
        <span>{label}</span>
        <span>{Math.round(safe * 100)}%</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-sunken" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={1} aria-valuenow={safe}>
        <div className={cn("h-full rounded-full transition-[width]", color)} style={{ width: `${Math.max(4, Math.round(safe * 100))}%` }} />
      </div>
    </div>
  );
}

function dictationStatusLabel(state: DictationState | undefined): string {
  switch (state) {
    case "transcribing":
      return "Transcribing";
    case "pasting":
      return "Pasting";
    case "error":
      return "Needs attention";
    default:
      return "Release to finish";
  }
}
