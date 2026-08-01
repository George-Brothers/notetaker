/**
 * The left rail: what you have, and how to get to it.
 *
 * This replaces what used to be two separate columns — a view picker, then a
 * list. Granola's insight is that those are the same thing (the list *is* the
 * navigation), and collapsing them buys the note itself the width it needs to
 * read like a document rather than a panel.
 *
 * Recordings are grouped by day, newest first, because that is how people look
 * for a meeting: "the one from Tuesday", never "the 14th of 63".
 */

import { useMemo, useState } from "react";
import type { FormEvent, KeyboardEvent, ReactNode } from "react";
import {
  ChevronDown,
  ChevronRight,
  FolderOpen,
  Inbox,
  Layers,
  NotebookPen,
  Plus,
  Search,
  Sparkles,
} from "lucide-react";
import type { RecordingRow, SearchHit } from "../lib/ipc";
import type { LibraryView } from "../hooks/useLibrary";
import { dayLabel, roughDuration, timeOfDay } from "../lib/format";
import { cn } from "../lib/cn";
import { Kbd, modKey } from "./ui";
import { StatusChip } from "./StatusChip";

export interface SidebarProps {
  tasks: string[];
  activeView: LibraryView;
  onSelectView: (view: LibraryView) => void;
  onCreateTask: (name: string) => void;
  recordings: RecordingRow[];
  selectedId: string | null;
  onSelectRecording: (id: string) => void;
  query: string;
  onSearch: (q: string) => void;
  searchResults: SearchHit[] | null;
  onOpenPalette: () => void;
  /** True while a queued recording cannot process because speech models are absent. */
  modelsMissing?: boolean;
  /** Layout only — the shell decides whether the rail is showing. */
  className?: string;
}

function isActive(a: LibraryView, b: LibraryView): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "task" && b.kind === "task") return a.name === b.name;
  return true;
}

/** Groups rows into day buckets, preserving the newest-first order. */
function groupByDay(rows: RecordingRow[]): Array<{ label: string; rows: RecordingRow[] }> {
  // One `now` for the whole render, so two rows either side of midnight cannot
  // disagree about which day "Today" is.
  const now = new Date();
  const out: Array<{ label: string; rows: RecordingRow[] }> = [];
  for (const row of rows) {
    const label = dayLabel(row.created, now);
    const last = out[out.length - 1];
    if (last && last.label === label) last.rows.push(row);
    else out.push({ label, rows: [row] });
  }
  return out;
}

function NavItem({
  active,
  icon,
  children,
  onClick,
}: {
  active: boolean;
  icon: ReactNode;
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-[var(--radius-control)] px-2 py-1.5 text-left text-[13px] transition-colors",
        active ? "bg-selected font-medium text-fg" : "text-fg-muted hover:bg-hover hover:text-fg",
      )}
    >
      <span className="shrink-0 text-fg-faint">{icon}</span>
      <span className="truncate">{children}</span>
    </button>
  );
}

function RecordingItem({
  row,
  selected,
  onSelect,
  modelsMissing,
}: {
  row: RecordingRow;
  selected: boolean;
  onSelect: () => void;
  modelsMissing: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex w-full flex-col gap-0.5 rounded-[var(--radius-control)] px-2 py-1.5 text-left transition-colors",
        selected ? "bg-selected" : "hover:bg-hover",
      )}
    >
      <span className="flex items-center gap-1.5">
        <span className={cn("min-w-0 flex-1 truncate text-[13px] text-fg", selected && "font-medium")}>
          {row.title}
        </span>
        {row.hasNotes && (
          <NotebookPen size={12} className="shrink-0 text-fg-faint" aria-label="Has your notes" />
        )}
        {row.suggestedTitle && (
          <Sparkles size={12} className="shrink-0 text-accent" aria-label="A better title is suggested" />
        )}
      </span>
      <span className="flex items-center gap-1.5 text-[11px] text-fg-faint">
        <span>{timeOfDay(row.created)}</span>
        <span aria-hidden>·</span>
        <span>{roughDuration(row.durationS)}</span>
        {row.status !== "ready" && (
          <>
            <span aria-hidden>·</span>
            <StatusChip status={row.status} compact />
          </>
        )}
      </span>
      {/* A failed row explains itself without being opened. `error` is written
          for a non-engineer by the runtime, and hiding it behind a click is how
          a recording silently looks like it just never processed. */}
      {row.status === "failed" && row.error && (
        <span className="text-[11px] leading-snug text-error">{row.error}</span>
      )}
      {row.status === "queued" && modelsMissing && (
        <span className="text-[11px] leading-snug text-fg-muted">Waiting on the speech models</span>
      )}
    </button>
  );
}

export function Sidebar({
  tasks,
  activeView,
  onSelectView,
  onCreateTask,
  recordings,
  selectedId,
  onSelectRecording,
  query,
  onSearch,
  searchResults,
  onOpenPalette,
  modelsMissing = false,
  className,
}: SidebarProps) {
  const [tasksOpen, setTasksOpen] = useState(true);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");
  const groups = useMemo(() => groupByDay(recordings), [recordings]);
  const searching = query.trim().length > 0;

  function submitCreate(e: FormEvent) {
    e.preventDefault();
    const trimmed = draft.trim();
    if (!trimmed) return;
    onCreateTask(trimmed);
    setDraft("");
    setCreating(false);
  }

  function handleDraftKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      setCreating(false);
      setDraft("");
    }
  }

  return (
    <nav
      aria-label="Library"
      className={cn(
        "h-full shrink-0 flex-col border-r border-border bg-app",
        className ?? "flex w-[264px]",
      )}
    >
      <div className="flex flex-col gap-1.5 px-3 pb-2 pt-3">
        <div className="relative">
          <Search
            size={14}
            aria-hidden
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-faint"
          />
          <input
            type="search"
            value={query}
            onChange={(e) => onSearch(e.target.value)}
            placeholder="Search everything"
            aria-label="Search transcripts and summaries"
            className="h-8 w-full rounded-[var(--radius-control)] border border-border bg-sunken pl-8 pr-2 text-[13px] text-fg placeholder:text-fg-faint focus:border-accent focus:outline-none"
          />
        </div>
        {/* A separate entry point from search: the palette also reaches
            commands and settings, and is the faster path once you know it. */}
        <button
          type="button"
          onClick={onOpenPalette}
          className="flex items-center justify-between rounded-[var(--radius-control)] px-1 py-0.5 text-[11px] text-fg-faint transition-colors hover:text-fg-muted"
        >
          <span>Jump to anything</span>
          <Kbd>{modKey()} K</Kbd>
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-4">
        {searching ? (
          <section aria-label="Search results">
            {searchResults === null ? (
              <p className="px-2 py-3 text-[13px] text-fg-muted">Searching…</p>
            ) : searchResults.length === 0 ? (
              <p className="px-2 py-3 text-[13px] text-fg-muted">
                No matches. Try a different word or phrase.
              </p>
            ) : (
              <ul className="flex flex-col gap-0.5">
                {searchResults.map((hit) => (
                  <li key={hit.id}>
                    <button
                      type="button"
                      onClick={() => onSelectRecording(hit.id)}
                      className={cn(
                        "flex w-full flex-col gap-0.5 rounded-[var(--radius-control)] px-2 py-1.5 text-left transition-colors",
                        hit.id === selectedId ? "bg-selected" : "hover:bg-hover",
                      )}
                    >
                      <span className="truncate text-[13px] text-fg">{hit.title}</span>
                      <span className="text-[11px] text-fg-faint">{hit.task ?? "Unsorted"}</span>
                      <span className="line-clamp-2 text-[12px] leading-snug text-fg-muted">
                        {/* SQLite's snippet() wraps matches in <b>. Rendered as
                            text rather than HTML: the tags are noise, and
                            injecting markup here would be the one XSS surface
                            in an app that otherwise renders no HTML at all. */}
                        {hit.snippet.replace(/<\/?b>/g, "")}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        ) : (
          <>
            <section aria-label="Views" className="flex flex-col gap-0.5">
              <NavItem
                active={isActive(activeView, { kind: "all" })}
                icon={<Layers size={14} />}
                onClick={() => onSelectView({ kind: "all" })}
              >
                All recordings
              </NavItem>
              <NavItem
                active={isActive(activeView, { kind: "unsorted" })}
                icon={<Inbox size={14} />}
                onClick={() => onSelectView({ kind: "unsorted" })}
              >
                Unsorted
              </NavItem>
              <NavItem
                active={isActive(activeView, { kind: "recent" })}
                icon={<Sparkles size={14} />}
                onClick={() => onSelectView({ kind: "recent" })}
              >
                Recently processed
              </NavItem>
            </section>

            <section aria-label="Tasks" className="mt-4">
              <div className="flex items-center justify-between px-2 pb-1">
                <button
                  type="button"
                  onClick={() => setTasksOpen((o) => !o)}
                  aria-expanded={tasksOpen}
                  className="flex items-center gap-1 text-[11px] font-semibold uppercase tracking-wide text-fg-faint hover:text-fg-muted"
                >
                  {tasksOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                  Tasks
                </button>
                <button
                  type="button"
                  onClick={() => setCreating(true)}
                  aria-label="New task"
                  className="rounded p-0.5 text-fg-faint hover:bg-hover hover:text-fg"
                >
                  <Plus size={13} />
                </button>
              </div>
              {tasksOpen && (
                <div className="flex flex-col gap-0.5">
                  {tasks.map((task) => (
                    <NavItem
                      key={task}
                      active={isActive(activeView, { kind: "task", name: task })}
                      icon={<FolderOpen size={14} />}
                      onClick={() => onSelectView({ kind: "task", name: task })}
                    >
                      {task}
                    </NavItem>
                  ))}
                  {tasks.length === 0 && !creating && (
                    <p className="px-2 py-1 text-[12px] text-fg-faint">
                      No tasks yet. Recordings stay in Unsorted until you make one.
                    </p>
                  )}
                  {creating && (
                    <form onSubmit={submitCreate} className="flex gap-1 px-2 py-1">
                      <label htmlFor="new-task-input" className="sr-only">
                        New task name
                      </label>
                      <input
                        id="new-task-input"
                        autoFocus
                        value={draft}
                        placeholder="Task name"
                        onChange={(e) => setDraft(e.target.value)}
                        onKeyDown={handleDraftKeyDown}
                        className="h-7 min-w-0 flex-1 rounded-[var(--radius-control)] border border-border bg-raised px-2 text-[13px] text-fg placeholder:text-fg-faint focus:border-accent focus:outline-none"
                      />
                      <button
                        type="submit"
                        className="rounded-[var(--radius-control)] px-2 text-[12px] font-medium text-accent hover:bg-hover"
                      >
                        Create
                      </button>
                    </form>
                  )}
                </div>
              )}
            </section>

            <section aria-label="Recordings" className="mt-4">
              {recordings.length === 0 ? (
                <p className="px-2 py-3 text-[12px] leading-relaxed text-fg-faint">
                  Nothing here yet. Hit record and start typing — your notes and the transcript land
                  here together.
                </p>
              ) : (
                groups.map((group) => (
                  <div key={group.label} className="mb-3">
                    <h2 className="px-2 pb-1 text-[11px] font-semibold uppercase tracking-wide text-fg-faint">
                      {group.label}
                    </h2>
                    <ul className="flex flex-col gap-0.5">
                      {group.rows.map((row) => (
                        <li key={row.id}>
                          <RecordingItem
                            row={row}
                            selected={row.id === selectedId}
                            onSelect={() => onSelectRecording(row.id)}
                            modelsMissing={modelsMissing}
                          />
                        </li>
                      ))}
                    </ul>
                  </div>
                ))
              )}
            </section>
          </>
        )}
      </div>
    </nav>
  );
}
