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
    // Bottom-right rather than centred: a meeting has just started, so the
    // thing this covers up is the thing you are trying to join.
    <div className="pointer-events-none fixed inset-0 z-50 flex items-end justify-end p-4">
      <div
        className="pointer-events-auto w-[22rem] rounded-[var(--radius-card)] border border-border bg-raised p-4 shadow-[var(--shadow-pop)]"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="meeting-prompt-title"
        ref={dialogRef}
        onKeyDown={trapFocus}
      >
        <p id="meeting-prompt-title" className="mb-3 text-[15px] font-semibold text-fg">
          {event.appName} started — record this?
        </p>
        <div className="flex flex-col gap-1.5 [&_button]:h-8 [&_button]:rounded-[var(--radius-control)] [&_button]:px-3 [&_button]:text-[13px] [&_button]:font-medium [&_button]:transition-colors [&_button:not(.meeting-prompt__record)]:border [&_button:not(.meeting-prompt__record)]:border-border [&_button:not(.meeting-prompt__record)]:text-fg-muted [&_button:not(.meeting-prompt__record)]:hover:bg-hover [&_.meeting-prompt__record]:bg-accent [&_.meeting-prompt__record]:text-accent-fg [&_.meeting-prompt__record]:hover:bg-accent-hover">
          {/* "Record this meeting", not "Record": the record control in the
              top strip is also called Record, and two buttons with the same
              accessible name on screen at once is ambiguous — for a screen
              reader reading them out, and for anyone glancing at the dialog. */}
          <button
            type="button"
            ref={recordButtonRef}
            className="meeting-prompt__record"
            onClick={onRecord}
          >
            Record this meeting
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
