/**
 * One rebindable shortcut. Click → listening state → next chord is captured.
 * Escape cancels. The conflict message arrives from above (registration is
 * the native layer's job); this component only renders it, verbatim.
 */
import { useState } from "react";
import type { KeyboardEvent } from "react";
import { acceleratorFromEvent, formatAcceleratorParts } from "../lib/hotkeys";
import { Kbd } from "./ui";
import { cn } from "../lib/cn";

// Design spec §5 "Hotkey recorder behavior" — the one approved string for
// the listening state. Held once so the visible label and the button's
// accessible name (below) can never drift apart.
const LISTENING_LABEL = "Press the keys…";

export function HotkeyField({
  label,
  hint,
  value,
  issue,
  onChange,
}: {
  label: string;
  hint: string;
  value: string;
  issue: string | null;
  onChange: (accelerator: string) => void;
}) {
  const [listening, setListening] = useState(false);

  function onKeyDown(e: KeyboardEvent<HTMLButtonElement>) {
    if (!listening) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setListening(false);
      return;
    }
    const accel = acceleratorFromEvent(e);
    if (accel) {
      onChange(accel);
      setListening(false);
    }
  }

  return (
    <div
      role="group"
      aria-label={label}
      className="flex items-center justify-between gap-4 rounded-[var(--radius-control)] border border-border bg-raised px-3 py-2.5"
    >
      <span className="min-w-0">
        <span className="block text-[13.5px] font-medium text-fg">{label}</span>
        <span className="block text-[12.5px] text-fg-muted">{hint}</span>
        {issue && (
          <span role="alert" className="block pt-1 text-[12.5px] text-error">
            {issue}
          </span>
        )}
      </span>
      <button
        type="button"
        aria-label={listening ? LISTENING_LABEL : `Change shortcut: ${label}`}
        onClick={() => setListening(true)}
        onKeyDown={onKeyDown}
        onBlur={() => setListening(false)}
        className={cn(
          "flex shrink-0 items-center gap-1 rounded-[var(--radius-control)] px-1.5 py-1",
          listening && "outline outline-2 outline-accent shadow-[var(--glow-accent)]",
        )}
      >
        {listening ? (
          <span className="text-[12.5px] text-accent">{LISTENING_LABEL}</span>
        ) : (
          formatAcceleratorParts(value).map((part) => <Kbd key={part}>{part}</Kbd>)
        )}
      </button>
    </div>
  );
}
