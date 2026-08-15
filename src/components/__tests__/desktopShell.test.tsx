import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { DesktopShell, effectivePaneSizes, layoutModeForWidth } from "../DesktopShell";
import type { PaneSizes } from "../DesktopShell";
import { RecordBar } from "../RecordBar";
import { NoteView } from "../NoteView";
import { TooltipProvider } from "../ui";
import type { CaptureStatus, RecordingDetail } from "../../lib/ipc";

const ORIGINAL_INNER_WIDTH = window.innerWidth;

function setViewport(width: number, height: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: height });
  window.dispatchEvent(new Event("resize"));
}

function renderShell({
  askOpen = true,
  mobilePane = "primary",
  initialLibraryWidth,
  initialAskWidth,
  onPaneSizesCommit = vi.fn(),
}: {
  askOpen?: boolean;
  mobilePane?: "library" | "primary";
  initialLibraryWidth?: number;
  initialAskWidth?: number;
  onPaneSizesCommit?: (sizes: PaneSizes) => void;
} = {}) {
  return render(
    <DesktopShell
      mobilePane={mobilePane}
      askOpen={askOpen}
      onAskOpenChange={vi.fn()}
      initialLibraryWidth={initialLibraryWidth}
      initialAskWidth={initialAskWidth}
      onPaneSizesCommit={onPaneSizesCommit}
      library={<div>Library</div>}
      primary={<main>Primary</main>}
      ask={
        <div>
          <button type="button" data-ask-initial-focus>
            Close Ask
          </button>
          <button type="button">Second Ask control</button>
        </div>
      }
    />,
  );
}

beforeEach(() => setViewport(1440, 900));

afterEach(() => {
  cleanup();
  setViewport(ORIGINAL_INNER_WIDTH, 900);
});

describe("DesktopShell layout modes", () => {
  it("keeps library, primary, and Ask in one three-pane desktop grid at 1440x900", () => {
    renderShell();
    const shell = screen.getByTestId("desktop-shell");

    expect(shell).toHaveAttribute("data-layout-mode", "desktop");
    expect(shell).toHaveAttribute("data-ask-open", "true");
    expect(screen.getByRole("separator", { name: "Resize library pane" })).toHaveAttribute(
      "aria-valuenow",
      "264",
    );
    expect(screen.getByRole("separator", { name: "Resize Ask pane" })).toHaveAttribute(
      "aria-valuenow",
      "340",
    );
    expect(screen.getByText("Library")).toBeVisible();
    expect(screen.getByText("Primary")).toBeVisible();
    expect(screen.getByText("Close Ask")).toBeVisible();
  });

  it("retains a 480px primary pane at constrained 1024x768 desktop", () => {
    setViewport(1024, 768);
    renderShell();
    const shell = screen.getByTestId("desktop-shell");
    const library = Number(screen.getByRole("separator", { name: "Resize library pane" }).getAttribute("aria-valuenow"));
    const ask = Number(screen.getByRole("separator", { name: "Resize Ask pane" }).getAttribute("aria-valuenow"));

    expect(shell).toHaveAttribute("data-layout-mode", "desktop");
    expect(library).toBeGreaterThanOrEqual(220);
    expect(ask).toBeGreaterThanOrEqual(300);
    expect(1024 - library - ask).toBeGreaterThanOrEqual(480);
  });

  it("turns Ask into a focus-contained drawer at 900x700", () => {
    setViewport(900, 700);
    const onAskOpenChange = vi.fn();
    render(
      <DesktopShell
        mobilePane="primary"
        askOpen
        onAskOpenChange={onAskOpenChange}
        library={<div>Library</div>}
        primary={<main>Primary</main>}
        ask={
          <div>
            <button type="button" data-ask-initial-focus>
              Close Ask
            </button>
            <button type="button">Second Ask control</button>
          </div>
        }
      />,
    );

    const shell = screen.getByTestId("desktop-shell");
    const drawer = screen.getByRole("dialog", { name: "Ask about this recording" });
    expect(shell).toHaveAttribute("data-layout-mode", "ask-overlay");
    expect(drawer).toHaveAttribute("aria-modal", "true");
    expect(screen.queryByRole("separator", { name: "Resize Ask pane" })).not.toBeInTheDocument();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close Ask" }));

    fireEvent.keyDown(drawer, { key: "Escape" });
    expect(onAskOpenChange).toHaveBeenCalledWith(false);
  });

  it("keeps the 720px boundary in the two-pane Ask-overlay mode", () => {
    setViewport(720, 700);
    renderShell();
    expect(screen.getByTestId("desktop-shell")).toHaveAttribute("data-layout-mode", "ask-overlay");
    expect(screen.queryByRole("separator", { name: "Resize library pane" })).not.toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Ask about this recording" })).toBeInTheDocument();
    expect(layoutModeForWidth(719)).toBe("single-pane");
    expect(layoutModeForWidth(720)).toBe("ask-overlay");
  });

  it("uses one navigation pane at 600x700 and removes the hidden pane from tab order", () => {
    setViewport(600, 700);
    renderShell({ askOpen: false, mobilePane: "library" });
    const shell = screen.getByTestId("desktop-shell");
    const primary = document.getElementById("primary-pane");

    expect(shell).toHaveAttribute("data-layout-mode", "single-pane");
    expect(screen.getByRole("region", { name: "Library" })).not.toHaveAttribute("aria-hidden", "true");
    expect(primary).toHaveAttribute("hidden");
    expect(primary).toHaveAttribute("aria-hidden", "true");
  });

  it("restores pane preferences with out-of-range values clamped to safe limits", async () => {
    const onPaneSizesCommit = vi.fn();
    renderShell({ initialLibraryWidth: 999, initialAskWidth: 1, onPaneSizesCommit });

    expect(screen.getByRole("separator", { name: "Resize library pane" })).toHaveAttribute(
      "aria-valuenow",
      "360",
    );
    expect(screen.getByRole("separator", { name: "Resize Ask pane" })).toHaveAttribute(
      "aria-valuenow",
      "300",
    );
    expect(onPaneSizesCommit).toHaveBeenCalledWith({ library: 360, ask: 300 });
  });

  it("resizes panes from the keyboard with named range values and a visible step", () => {
    const onPaneSizesCommit = vi.fn();
    renderShell({ onPaneSizesCommit });
    const separator = screen.getByRole("separator", { name: "Resize library pane" });

    expect(separator).toHaveAttribute("aria-valuemin", "220");
    expect(separator).toHaveAttribute("aria-valuemax", "360");
    expect(separator).toHaveAttribute("data-keyboard-step", "16");
    fireEvent.keyDown(separator, { key: "ArrowRight" });

    expect(separator).toHaveAttribute("aria-valuenow", "280");
    expect(onPaneSizesCommit).toHaveBeenLastCalledWith({ library: 280, ask: 340 });
  });

  it("keeps Ask closed out of the grid until the user opens it", () => {
    const { rerender } = renderShell({ askOpen: false });
    expect(screen.getByTestId("desktop-shell")).toHaveAttribute("data-ask-open", "false");
    expect(screen.queryByRole("region", { name: "Ask about this recording" })).not.toBeInTheDocument();

    rerender(
      <DesktopShell
        mobilePane="primary"
        askOpen
        onAskOpenChange={vi.fn()}
        library={<div>Library</div>}
        primary={<main>Primary</main>}
        ask={<div>Ask content</div>}
      />,
    );
    expect(screen.getByTestId("desktop-shell")).toHaveAttribute("data-ask-open", "true");
    expect(screen.getByRole("region", { name: "Ask about this recording" })).toBeInTheDocument();
  });
});

describe("responsive content geometry", () => {
  it("keeps long recording status and duration readable without hiding Stop", () => {
    const status: CaptureStatus = {
      state: "recording",
      mode: "meeting",
      recordingId: "long-recording",
      elapsedS: 86_399,
      micLevel: 0.5,
      systemLevel: 0.4,
      diskFreeMb: 20_000,
    };
    render(
      <TooltipProvider>
        <RecordBar status={status} onStart={vi.fn()} onPause={vi.fn()} onResume={vi.fn()} onStop={vi.fn()} />
      </TooltipProvider>,
    );

    expect(screen.getByLabelText("Elapsed recording time")).toHaveTextContent("23:59:59");
    expect(screen.getByRole("button", { name: "Stop" })).toBeVisible();
    expect(document.querySelector(".record-bar__main")).toBeInTheDocument();
    expect(document.querySelector(".record-bar__meters")).toBeInTheDocument();
  });

  it("puts a long saving status on the compact status surface", () => {
    const status: CaptureStatus = {
      state: "finishing",
      mode: null,
      recordingId: "long-saving-recording",
      elapsedS: 86_399,
      micLevel: 0,
      systemLevel: 0,
      diskFreeMb: 20_000,
    };
    render(
      <TooltipProvider>
        <RecordBar status={status} onStart={vi.fn()} onPause={vi.fn()} onResume={vi.fn()} onStop={vi.fn()} />
      </TooltipProvider>,
    );

    const statusLine = screen.getByRole("status");
    expect(statusLine).toHaveClass("record-bar__status");
    expect(statusLine).toHaveTextContent(/Saving your recording/);
  });

  it("limits a long meeting title to the shared note-title surface", () => {
    const title = "A very long meeting title that should not push the document controls off screen";
    const detail: RecordingDetail = {
      id: "rec-long-title",
      title,
      task: null,
      created: "2026-08-15T14:00:00Z",
      durationS: 90,
      mode: "meeting",
      status: "ready",
      hasNotes: false,
      transcriptMd: "",
      summaryMd: "",
      notesMd: "",
      error: null,
      captureNote: null,
      suggestedTask: null,
      suggestedTitle: null,
      template: null,
      actions: [],
      speakers: {},
      segments: [],
      audioTracks: [],
    };
    render(
      <TooltipProvider>
        <NoteView
          detail={detail}
          loading={false}
          tasks={[]}
          templates={[]}
          askOpen={false}
          onToggleAsk={vi.fn()}
          onRenameSpeaker={vi.fn()}
          onSaveSummary={vi.fn()}
          onRenameRecording={vi.fn()}
          onAssignTask={vi.fn()}
          onSaveNotes={vi.fn().mockResolvedValue(undefined)}
          onSetTemplate={vi.fn()}
          onToggleAction={vi.fn()}
          onProcessNow={vi.fn()}
          liveRecordingId={null}
          onBack={vi.fn()}
        />
      </TooltipProvider>,
    );

    expect(screen.getByRole("button", { name: title })).toHaveClass("note-view__title");
    expect(screen.getByRole("button", { name: /Ask$/ })).toBeVisible();
  });
});

describe("pane fit calculation", () => {
  it("leaves the configured native minimum width usable as one pane", () => {
    expect(layoutModeForWidth(560)).toBe("single-pane");
    expect(effectivePaneSizes(560, { library: 264, ask: 340 }, false)).toEqual({
      library: 264,
      ask: 340,
    });
  });
});
