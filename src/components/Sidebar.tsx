import { useState } from "react";
import type { FormEvent, KeyboardEvent } from "react";
import type { LibraryView } from "../hooks/useLibrary";

export interface SidebarProps {
  tasks: string[];
  activeView: LibraryView;
  onSelectView: (view: LibraryView) => void;
  onCreateTask: (name: string) => void;
}

const FIXED_VIEWS: Array<{ view: LibraryView; label: string }> = [
  { view: { kind: "all" }, label: "All" },
  { view: { kind: "unsorted" }, label: "Unsorted" },
  { view: { kind: "recent" }, label: "Recently processed" },
];

function isActive(a: LibraryView, b: LibraryView): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "task" && b.kind === "task") return a.name === b.name;
  return true;
}

export function Sidebar({ tasks, activeView, onSelectView, onCreateTask }: SidebarProps) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");

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
    <nav className="sidebar" aria-label="Library">
      <ul className="sidebar__views">
        {FIXED_VIEWS.map(({ view, label }) => (
          <li key={label}>
            <button
              type="button"
              className={`sidebar__item${isActive(activeView, view) ? " sidebar__item--active" : ""}`}
              aria-current={isActive(activeView, view) ? "true" : undefined}
              onClick={() => onSelectView(view)}
            >
              {label}
            </button>
          </li>
        ))}
      </ul>

      <div className="sidebar__tasks">
        <h3 className="sidebar__heading">Tasks</h3>
        <ul className="sidebar__views">
          {tasks.map((task) => {
            const view: LibraryView = { kind: "task", name: task };
            return (
              <li key={task}>
                <button
                  type="button"
                  className={`sidebar__item${isActive(activeView, view) ? " sidebar__item--active" : ""}`}
                  aria-current={isActive(activeView, view) ? "true" : undefined}
                  onClick={() => onSelectView(view)}
                >
                  {task}
                </button>
              </li>
            );
          })}
        </ul>

        {creating ? (
          <form className="sidebar__new-task" onSubmit={submitCreate}>
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
            />
            <button type="submit">Create</button>
            <button
              type="button"
              onClick={() => {
                setCreating(false);
                setDraft("");
              }}
            >
              Cancel
            </button>
          </form>
        ) : (
          <button type="button" className="sidebar__add-task" onClick={() => setCreating(true)}>
            + New task
          </button>
        )}
      </div>
    </nav>
  );
}
