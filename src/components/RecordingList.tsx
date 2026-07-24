import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { Mode, RecordingRow } from "../lib/ipc";
import { StatusChip } from "./StatusChip";

export interface RecordingListProps {
  recordings: RecordingRow[];
  tasks: string[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAssignTask: (id: string, task: string) => void;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const ss = String(sec).padStart(2, "0");
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
  return `${m}:${ss}`;
}

function ModeGlyph({ mode }: { mode: Mode }) {
  return mode === "meeting" ? (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" className="mode-glyph">
      <circle cx="5" cy="6" r="2.4" fill="currentColor" opacity="0.55" />
      <circle cx="10" cy="6" r="2.4" fill="currentColor" />
      <path
        d="M2 13c.6-2.4 2.3-3.6 3.5-3.6M14 13c-.6-2.6-2.6-3.9-4-3.9"
        stroke="currentColor"
        strokeWidth="1.2"
        fill="none"
        strokeLinecap="round"
      />
    </svg>
  ) : (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" className="mode-glyph">
      <circle cx="8" cy="5.5" r="2.6" fill="currentColor" />
      <path
        d="M3 13c.8-3 3-4.4 5-4.4s4.2 1.4 5 4.4"
        stroke="currentColor"
        strokeWidth="1.2"
        fill="none"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** RecordingRow doesn't declare `error` in the ipc.ts contract, but a
 * failed row must surface its reason (brief requirement); read it
 * defensively in case the Rust side includes it on the list endpoint. */
function rowError(row: RecordingRow): string | null {
  const withError = row as RecordingRow & { error?: string | null };
  return withError.error ?? null;
}

export function RecordingList({ recordings, tasks, selectedId, onSelect, onAssignTask }: RecordingListProps) {
  const [focusedIndex, setFocusedIndex] = useState(0);
  const [choosingFor, setChoosingFor] = useState<string | null>(null);
  const itemRefs = useRef<Array<HTMLLIElement | null>>([]);

  useEffect(() => {
    if (focusedIndex > recordings.length - 1) {
      setFocusedIndex(Math.max(0, recordings.length - 1));
    }
  }, [recordings.length, focusedIndex]);

  useEffect(() => {
    itemRefs.current[focusedIndex]?.focus();
  }, [focusedIndex]);

  if (recordings.length === 0) {
    return <p className="empty-state">Nothing here yet. Recordings will show up once they're captured.</p>;
  }

  function handleKeyDown(e: ReactKeyboardEvent<HTMLUListElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setFocusedIndex((i) => Math.min(i + 1, recordings.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setFocusedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter" || e.key === " ") {
      const row = recordings[focusedIndex];
      if (row) {
        e.preventDefault();
        onSelect(row.id);
      }
    }
  }

  return (
    // Deliberately not role="listbox"/"option": each row nests interactive
    // controls (Accept, Choose different, the task <select>), and ARIA
    // authoring practice disallows focusable descendants inside an
    // "option". A plain list with roving tabindex + aria-current stays
    // keyboard-operable without that conflict.
    <ul className="rec-list" aria-label="Recordings" onKeyDown={handleKeyDown}>
      {recordings.map((row, index) => {
        const error = rowError(row);
        return (
          <li
            key={row.id}
            ref={(el) => {
              itemRefs.current[index] = el;
            }}
            aria-current={row.id === selectedId ? "true" : undefined}
            tabIndex={index === focusedIndex ? 0 : -1}
            className={`rec-row${row.id === selectedId ? " rec-row--selected" : ""}`}
            onClick={() => {
              setFocusedIndex(index);
              onSelect(row.id);
            }}
          >
            <div className="rec-row__main">
              <ModeGlyph mode={row.mode} />
              <div className="rec-row__text">
                <span className="rec-row__title">{row.title}</span>
                <span className="rec-row__meta">
                  {formatDate(row.created)} · {formatDuration(row.durationS)} ·{" "}
                  {row.mode === "meeting" ? "Meeting" : "In person"}
                </span>
              </div>
              <StatusChip status={row.status} error={error} />
            </div>

            {row.suggestedTask && (
              <div className="suggestion-banner" onKeyDown={(e) => e.stopPropagation()}>
                <span className="suggestion-banner__text">Suggested: {row.suggestedTask}</span>
                <button
                  type="button"
                  className="suggestion-banner__accept"
                  onClick={(e) => {
                    e.stopPropagation();
                    onAssignTask(row.id, row.suggestedTask as string);
                  }}
                >
                  Accept
                </button>
                <button
                  type="button"
                  className="suggestion-banner__change"
                  onClick={(e) => {
                    e.stopPropagation();
                    setChoosingFor(choosingFor === row.id ? null : row.id);
                  }}
                >
                  Choose different…
                </button>
                {choosingFor === row.id && (
                  <select
                    aria-label={`Choose a different task for ${row.title}`}
                    autoFocus
                    defaultValue=""
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => {
                      e.stopPropagation();
                      if (e.target.value) {
                        onAssignTask(row.id, e.target.value);
                        setChoosingFor(null);
                      }
                    }}
                  >
                    <option value="" disabled>
                      Pick a task…
                    </option>
                    {tasks.map((t) => (
                      <option key={t} value={t}>
                        {t}
                      </option>
                    ))}
                  </select>
                )}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
