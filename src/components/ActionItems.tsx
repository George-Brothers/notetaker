/**
 * The checklist, lifted from Circleback and Fireflies.
 *
 * The items are not stored anywhere of their own — they are the checkbox lines
 * already inside `summary.md`, so ticking one edits that line and the list can
 * never drift from the document the user can edit by hand. See `core/actions.rs`.
 *
 * Ticking is optimistic: the box moves on click and the command returns the
 * whole re-parsed list, which replaces it. A checkbox that waits for a disk
 * write before moving feels broken even when the write takes 8 ms.
 */

import { useState } from "react";
import { Check, ListChecks } from "lucide-react";
import type { ActionItem } from "../lib/ipc";
import { cn } from "../lib/cn";

export function ActionItems({
  items,
  onToggle,
}: {
  items: ActionItem[];
  onToggle: (index: number, done: boolean) => void;
}) {
  // Indices the user just clicked, shown in their new state while the write is
  // in flight. Cleared when `items` comes back changed, which happens on the
  // next render after the command resolves.
  const [optimistic, setOptimistic] = useState<Record<number, boolean>>({});

  if (items.length === 0) return null;

  const done = items.filter((i) => (optimistic[i.index] ?? i.done)).length;

  return (
    <section aria-label="Action items" className="my-6">
      <header className="mb-2 flex items-center gap-2">
        <ListChecks size={14} className="text-fg-faint" aria-hidden />
        <h2 className="text-[13px] font-semibold uppercase tracking-wide text-fg-muted">
          Action items
        </h2>
        <span className="text-[12px] text-fg-faint">
          {done} of {items.length}
        </span>
      </header>

      <ul className="flex flex-col">
        {items.map((item) => {
          const checked = optimistic[item.index] ?? item.done;
          // The owner prefix is shown as its own chip, so strip it from the
          // body — "Alice  Alice: send the deck" reads badly.
          const body = item.owner
            ? item.text.slice(item.text.indexOf(":") + 1).trim()
            : item.text;
          return (
            <li key={item.index}>
              <label
                className={cn(
                  "flex cursor-pointer items-start gap-2.5 rounded-[var(--radius-control)] px-2 py-1.5 transition-colors hover:bg-hover",
                )}
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={(e) => {
                    const next = e.target.checked;
                    setOptimistic((o) => ({ ...o, [item.index]: next }));
                    onToggle(item.index, next);
                  }}
                  className="peer sr-only"
                />
                <span
                  aria-hidden
                  className={cn(
                    "mt-[3px] flex h-[15px] w-[15px] shrink-0 items-center justify-center rounded-[4px] border transition-colors",
                    checked ? "border-accent bg-accent text-accent-fg" : "border-border-strong bg-raised",
                    "peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent",
                  )}
                >
                  {checked && <Check size={11} strokeWidth={3} />}
                </span>
                <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  {item.owner && (
                    <span className="rounded-full bg-accent-soft px-1.5 py-0.5 text-[11px] font-medium text-accent">
                      {item.owner}
                    </span>
                  )}
                  <span
                    className={cn(
                      "text-[15px] leading-[1.6]",
                      checked ? "text-fg-faint line-through" : "text-fg",
                    )}
                  >
                    {body}
                  </span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
