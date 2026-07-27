import { useEffect, useRef } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { MeetingEvent } from "../lib/ipc";

export interface MeetingPromptProps {
  event: MeetingEvent;
  onRecord: () => void;
  onNotNow: () => void;
  onAlways: () => void;
  onNever: () => void;
}

/**
 * "Zoom started — record this?" — the ask-first prompt from the meeting
 * watcher (spec §4.2). Only ever rendered for a `kind: "started"` event
 * whose policy is "ask" (`autoStart: false`); an "always" policy starts
 * recording with no prompt, and this component never sees "ended" events.
 */
export function MeetingPrompt({ event, onRecord, onNotNow, onAlways, onNever }: MeetingPromptProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const recordButtonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    recordButtonRef.current?.focus();
  }, []);

  function trapFocus(e: ReactKeyboardEvent<HTMLDivElement>) {
    if (e.key === "Escape") {
      e.preventDefault();
      onNotNow();
      return;
    }
    if (e.key !== "Tab") return;
    const container = dialogRef.current;
    if (!container) return;
    const focusables = Array.from(container.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="meeting-prompt-overlay">
      <div
        className="meeting-prompt"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="meeting-prompt-title"
        ref={dialogRef}
        onKeyDown={trapFocus}
      >
        <p id="meeting-prompt-title" className="meeting-prompt__title">
          {event.appName} started — record this?
        </p>
        <div className="meeting-prompt__actions">
          <button
            type="button"
            ref={recordButtonRef}
            className="meeting-prompt__record"
            onClick={onRecord}
          >
            Record
          </button>
          <button type="button" onClick={onNotNow}>
            Not now
          </button>
          <button type="button" onClick={onAlways}>
            Always record {event.appName}
          </button>
          <button type="button" onClick={onNever}>
            Never ask for {event.appName}
          </button>
        </div>
      </div>
    </div>
  );
}
