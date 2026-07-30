/**
 * The live notepad — the thing that makes this a notebook rather than a
 * recorder.
 *
 * Two rules make autosave safe, and both are the kind that only show up when a
 * real person is mid-sentence:
 *
 * 1. **The textarea owns its text.** Server state seeds it when the *recording*
 *    changes and never again. A refetch that pushed `detail.notesMd` back into
 *    the box would overwrite whatever was typed since the request went out,
 *    which is how autosave eats a word.
 * 2. **A pending save is always flushed.** Switching recordings, closing the
 *    window, or the browser hiding the tab all fire before the debounce would
 *    have — so each one flushes rather than letting the timer die with the
 *    component.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../lib/cn";

/** Long enough not to write on every keystroke, short enough to feel saved. */
const AUTOSAVE_MS = 800;

export type SaveState = "idle" | "saving" | "saved";

export function Notepad({
  recordingId,
  initialNotes,
  onSave,
  placeholder = "Type your notes here while it records…",
  autoFocus,
  onStateChange,
}: {
  recordingId: string;
  initialNotes: string;
  onSave: (id: string, notesMd: string) => Promise<void>;
  placeholder?: string;
  autoFocus?: boolean;
  onStateChange?: (state: SaveState) => void;
}) {
  const [text, setText] = useState(initialNotes);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Everything the flush needs, in refs — so the unmount cleanup below can run
  // with an empty dependency list and still see the latest text. With `text` in
  // the deps the effect would tear down and re-arm on every keystroke, and the
  // flush would fire constantly instead of on the way out.
  const pending = useRef(false);
  const latest = useRef(text);
  const idRef = useRef(recordingId);
  const saveRef = useRef(onSave);
  latest.current = text;
  saveRef.current = onSave;

  const flush = useCallback(() => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    if (!pending.current) return;
    pending.current = false;
    const id = idRef.current;
    const body = latest.current;
    onStateChange?.("saving");
    void saveRef
      .current(id, body)
      .then(() => onStateChange?.("saved"))
      // The hook surfaces the failure; this only stops the indicator claiming
      // "Saved" over a write that did not happen.
      .catch(() => onStateChange?.("idle"));
  }, [onStateChange]);

  // A different recording means a different note. Flush the old one first —
  // switching away is the single most likely moment to lose an unsaved edit.
  useEffect(() => {
    if (idRef.current !== recordingId) {
      flush();
      idRef.current = recordingId;
      setText(initialNotes);
      latest.current = initialNotes;
      onStateChange?.("idle");
    }
  }, [recordingId, initialNotes, flush, onStateChange]);

  // The window going away, and the tab being hidden. `visibilitychange` is the
  // one that actually fires reliably on a phone, where the app is backgrounded
  // rather than closed.
  useEffect(() => {
    const onHide = () => flush();
    window.addEventListener("beforeunload", onHide);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      window.removeEventListener("beforeunload", onHide);
      document.removeEventListener("visibilitychange", onHide);
      flush();
    };
  }, [flush]);

  function change(next: string) {
    setText(next);
    latest.current = next;
    pending.current = true;
    onStateChange?.("idle");
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(flush, AUTOSAVE_MS);
  }

  return (
    <textarea
      value={text}
      onChange={(e) => change(e.target.value)}
      onBlur={flush}
      autoFocus={autoFocus}
      placeholder={placeholder}
      aria-label="Your notes"
      spellCheck
      className={cn(
        "w-full resize-none border-0 bg-transparent p-0 text-[15px] leading-[1.7] text-fg",
        "placeholder:text-fg-faint focus:outline-none",
        // Grows with its content rather than scrolling in a small box: this is
        // a document, not a form field.
        "field-sizing-content min-h-[8rem]",
      )}
    />
  );
}
