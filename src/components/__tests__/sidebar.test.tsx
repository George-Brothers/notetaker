import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "../Sidebar";
import type { SidebarProps } from "../Sidebar";
import type { RecordingRow } from "../../lib/ipc";

const queued: RecordingRow = {
  id: "queued-recording",
  title: "Lecture",
  task: null,
  created: "2026-08-01T12:00:00Z",
  durationS: 60,
  mode: "in_person",
  status: "queued",
  suggestedTask: null,
  suggestedTitle: null,
  error: null,
  captureNote: null,
  hasNotes: false,
};

/** RFC3339 for `days` calendar days before right now — 0 is today, 1 is yesterday. */
function daysAgo(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() - days);
  return d.toISOString();
}

const ROW_TODAY: RecordingRow = {
  ...queued,
  id: "row-today",
  title: "Today recording",
  created: daysAgo(0),
};
const ROW_YESTERDAY: RecordingRow = {
  ...queued,
  id: "row-yesterday",
  title: "Yesterday recording",
  created: daysAgo(1),
};

afterEach(() => {
  cleanup();
});

/** Renders `Sidebar` with sensible defaults; pass only what a test cares about. */
function renderSidebar(overrides: Partial<SidebarProps> = {}) {
  return render(
    <Sidebar
      tasks={[]}
      activeView={{ kind: "all" }}
      onSelectView={vi.fn()}
      onCreateTask={vi.fn()}
      recordings={[]}
      selectedId={null}
      onSelectRecording={vi.fn()}
      query=""
      onSearch={vi.fn()}
      searchResults={null}
      onOpenPalette={vi.fn()}
      sort="newest"
      onSetSort={vi.fn()}
      filter="all"
      onSetFilter={vi.fn()}
      {...overrides}
    />,
  );
}

describe("Sidebar setup status", () => {
  it("tells a queued recording why it is waiting when speech models are missing", () => {
    renderSidebar({ recordings: [queued], modelsMissing: true });

    expect(screen.getByText(/waiting on the speech models/i)).toBeInTheDocument();
  });
});

describe("sort and filter controls", () => {
  it("renders the sort control showing the active order", () => {
    renderSidebar({ sort: "newest" });
    expect(screen.getByLabelText("Sort recordings")).toHaveValue("newest");
  });

  it("changing sort calls onSetSort", () => {
    const onSetSort = vi.fn();
    renderSidebar({ onSetSort });
    fireEvent.change(screen.getByLabelText("Sort recordings"), {
      target: { value: "longest" },
    });
    expect(onSetSort).toHaveBeenCalledWith("longest");
  });

  it("changing filter calls onSetFilter", () => {
    const onSetFilter = vi.fn();
    renderSidebar({ onSetFilter });
    fireEvent.change(screen.getByLabelText("Show only"), {
      target: { value: "error" },
    });
    expect(onSetFilter).toHaveBeenCalledWith("error");
  });

  it("hides day headers when sorted by length", () => {
    renderSidebar({ sort: "longest", recordings: [ROW_TODAY, ROW_YESTERDAY] });
    expect(screen.queryByText("Today")).not.toBeInTheDocument();
  });
});
