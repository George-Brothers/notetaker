import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandPalette } from "../CommandPalette";
import type { RecordingRow } from "../../lib/ipc";

// jsdom has no ResizeObserver; cmdk's Command.List uses one purely to track
// its own rendered height for a CSS variable. Nothing below asserts on
// layout, so a no-op stub is enough to let the dialog mount.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

// jsdom also has no scrollIntoView; cmdk calls it once on mount to scroll the
// initially-selected row into view. Also layout-only, also a no-op here.
Element.prototype.scrollIntoView = vi.fn();

// vitest globals are off in this project (see vite.config.ts), so
// testing-library's auto-cleanup never engages on its own — every other test
// file in this repo calls this explicitly, and skipping it here would leak
// each render into the next test's DOM.
afterEach(() => {
  cleanup();
});

const row: RecordingRow = {
  id: "r1", title: "Accounting sync", task: null, created: "2026-08-04T10:00:00Z",
  durationS: 60, mode: "meeting", status: "ready", suggestedTask: null,
  suggestedTitle: null, hasNotes: false, error: null, captureNote: null,
};

function renderPalette(over: Partial<Parameters<typeof CommandPalette>[0]> = {}) {
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    recordings: [row],
    tasks: ["Entrepreneurship"],
    onSelectRecording: vi.fn(),
    onSelectTask: vi.fn(),
    onOpenSettings: vi.fn(),
    ...over,
  };
  render(<CommandPalette {...props} />);
  return props;
}

describe("CommandPalette", () => {
  it("never offers recording controls", () => {
    renderPalette();
    expect(screen.queryByText(/Record a meeting/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Record in person/)).not.toBeInTheDocument();
    expect(screen.queryByText("Stop recording", { exact: true })).not.toBeInTheDocument();
    expect(screen.queryByText("mode", { exact: true })).not.toBeInTheDocument();
  });

  it("jumps to a recording", () => {
    const p = renderPalette();
    fireEvent.click(screen.getByText("Accounting sync"));
    expect(p.onSelectRecording).toHaveBeenCalledWith("r1");
    expect(p.onOpenChange).toHaveBeenCalledWith(false);
  });

  it("jumps to a task", () => {
    const p = renderPalette();
    fireEvent.click(screen.getByText("Entrepreneurship"));
    expect(p.onSelectTask).toHaveBeenCalledWith("Entrepreneurship");
  });

  // cmdk only hides a Command.Group once its item count is filtered to zero
  // by an active search string; with the search box empty (the state right
  // after opening), a heading with no rows under it renders anyway. Confirmed
  // by reading cmdk 1.1.1's own visibility check before adding this guard —
  // it is not something the library does for you.
  it("hides the Tasks heading when there are no tasks", () => {
    renderPalette({ tasks: [] });
    expect(screen.queryByText("Tasks")).not.toBeInTheDocument();
  });

  it("deep-links into a settings section", () => {
    const p = renderPalette();
    fireEvent.click(screen.getByText("Shortcuts"));
    expect(p.onOpenSettings).toHaveBeenCalledWith("shortcuts");
  });
});
