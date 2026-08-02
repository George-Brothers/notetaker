import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { describe, expect, it, vi } from "vitest";
import { Sidebar } from "../Sidebar";
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

describe("Sidebar setup status", () => {
  it("tells a queued recording why it is waiting when speech models are missing", () => {
    render(
      <Sidebar
        tasks={[]}
        activeView={{ kind: "all" }}
        onSelectView={vi.fn()}
        onCreateTask={vi.fn()}
        recordings={[queued]}
        selectedId={null}
        onSelectRecording={vi.fn()}
        query=""
        onSearch={vi.fn()}
        searchResults={null}
        onOpenPalette={vi.fn()}
        modelsMissing
      />,
    );

    expect(screen.getByText(/waiting on the speech models/i)).toBeInTheDocument();
  });
});
