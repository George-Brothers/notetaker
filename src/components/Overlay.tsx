/**
 * The floating pill: recording state at a glance, always on top, in the
 * corner of whatever the meeting is happening over.
 *
 * This renders in its own webview (`index.html#overlay`) and is deliberately
 * a dumb terminal. The main window owns capture, settings, and the meeting
 * watcher; it pushes state here over one event ("overlay-sync") and this
 * window answers with intent events ("overlay-record", "overlay-pause-resume",
 * "overlay-stop", "overlay-open", "overlay-dismiss") that App handles with
 * exactly the code the tray menu uses. Two dumb remotes, one owner — polling
 * from here would instead race the main window for the meeting watcher's
 * drained events.
 *
 * The window itself is content-protected (invisible to screen shares) and
 * follows across workspaces — see the builder in src-tauri/src/lib.rs.
 */

import { useEffect, useState } from "react";
import { Mic, Pause, Play, Square, Star, X } from "lucide-react";
import { emit, listen } from "@tauri-apps/api/event";
import { useTheme } from "../hooks/useTheme";
import { duration } from "../lib/format";
import { cn } from "../lib/cn";

/** What the main window tells this pill to show. */
export interface OverlaySync {
  /** "prompt" = meeting detected, nothing recording yet. */
  kind: "recording" | "prompt";
  state: "recording" | "paused" | "finishing";
  elapsedS: number;
  /** The detected meeting app's display name, prompt mode only. */
  appName: string | null;
}

export function Overlay() {
  // Applies the stored theme to this webview's root, same as the main window.
  useTheme();
  const [sync, setSync] = useState<OverlaySync | null>(null);

  useEffect(() => {
    const un = listen<OverlaySync>("overlay-sync", (e) => setSync(e.payload));
    un.catch(() => {});
    return () => {
      void un.then((stop) => stop()).catch(() => {});
    };
  }, []);

  const send = (event: string) => () => void emit(event).catch(() => {});

  const iconBtn = cn(
    "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full",
    "text-fg-muted transition-colors hover:bg-hover hover:text-fg",
    "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent",
  );

  // Nothing yet: an empty pill would read as a broken one. The window is
  // normally hidden until the first sync, so this is a one-frame guard.
  if (!sync) return null;

  return (
    <div
      data-tauri-drag-region="deep"
      className="flex h-screen w-screen select-none items-center gap-2 overflow-hidden rounded-full border border-border bg-raised/95 px-3 shadow-lg"
    >
      {sync.kind === "prompt" ? (
        <>
          <Mic size={14} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1 truncate text-[12px] text-fg">
            Record {sync.appName ?? "this meeting"}?
          </span>
          <button
            type="button"
            onClick={send("overlay-record")}
            className="shrink-0 rounded-full bg-recording px-3 py-1 text-[12px] font-medium text-white transition-opacity hover:opacity-90"
          >
            Record
          </button>
          <button type="button" aria-label="Dismiss" className={iconBtn} onClick={send("overlay-dismiss")}>
            <X size={13} />
          </button>
        </>
      ) : (
        <>
          <span
            className={cn(
              "h-2 w-2 shrink-0 rounded-full",
              sync.state === "recording" ? "animate-pulse bg-recording" : "bg-fg-faint",
            )}
          />
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[12px] font-medium tabular-nums",
              sync.state === "recording" ? "text-recording" : "text-fg-muted",
            )}
          >
            {sync.state === "finishing" ? "Finishing…" : duration(sync.elapsedS)}
            {sync.state === "paused" && " — paused"}
          </span>
          {sync.state !== "finishing" && (
            <>
              <button
                type="button"
                aria-label="Star this moment"
                className={iconBtn}
                onClick={send("overlay-highlight")}
              >
                <Star size={13} />
              </button>
              <button
                type="button"
                aria-label={sync.state === "paused" ? "Resume" : "Pause"}
                className={iconBtn}
                onClick={send("overlay-pause-resume")}
              >
                {sync.state === "paused" ? <Play size={13} /> : <Pause size={13} />}
              </button>
              <button
                type="button"
                aria-label="Stop recording"
                className={cn(iconBtn, "hover:bg-recording hover:text-white")}
                onClick={send("overlay-stop")}
              >
                <Square size={12} />
              </button>
            </>
          )}
          <button
            type="button"
            aria-label="Open Notetaker"
            className={iconBtn}
            onClick={send("overlay-open")}
          >
            <Mic size={13} />
          </button>
        </>
      )}
    </div>
  );
}
