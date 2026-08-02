import { fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { describe, expect, it, vi } from "vitest";
import { QueuePanel } from "../QueuePanel";
import type { QueueSnapshot } from "../../lib/ipc";

const snapshot: QueueSnapshot = {
  items: [
    {
      id: "recording-1",
      title: "Planning call",
      state: "processing",
      stage: "Transcribing call audio",
      stageIndex: 4,
      stageCount: 8,
      startedAt: "2026-08-02T18:00:00Z",
      elapsedS: 42,
      position: null,
      attempts: 0,
      error: null,
      created: "2026-08-02T17:50:00Z",
    },
    {
      id: "recording-2",
      title: "Older note",
      state: "failed",
      stage: null,
      stageIndex: 0,
      stageCount: 0,
      startedAt: null,
      elapsedS: 2,
      position: null,
      attempts: 3,
      error: "The local model stopped responding.",
      created: "2026-08-02T17:00:00Z",
    },
  ],
  processingEnabled: true,
  idleAllowed: true,
  modelsReady: true,
};

describe("QueuePanel", () => {
  it("shows the current stage and exposes actionable retry controls", () => {
    const onRetry = vi.fn();
    render(
      <QueuePanel
        snapshot={snapshot}
        onSelectRecording={vi.fn()}
        onPause={vi.fn()}
        onResume={vi.fn()}
        onCancel={vi.fn()}
        onRetry={onRetry}
      />,
    );

    expect(screen.getAllByText(/Transcribing call audio/).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledWith("recording-2");
  });
});
