import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, within, fireEvent, waitFor, cleanup, act } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import App from "../../App";
import { api } from "../../lib/ipc";
import { applyIpcDefaults } from "../../test/ipcMock";
import type { CaptureStatus, RecordingDetail, RecordingRow } from "../../lib/ipc";

// Every api function App reaches for has to be stubbed, not just the ones
// these tests assert on. App also mounts the capture hook, which calls
// `captureStatus` and `pollMeetings` on mount and again on a timer; leaving
// them off the mock left them `undefined`, and the resulting throw set an
// error banner mid-test at an unpredictable moment — worth roughly one flaky
// failure in eight, in whichever test happened to be interacting at the time.
vi.mock("../../lib/ipc", async (importOriginal) => {
  // Keys derived from the real contract, so adding a command to ipc.ts can
  // never again turn every test in this file red. See src/test/ipcMock.ts.
  const actual = await importOriginal<typeof import("../../lib/ipc")>();
  return {
    ...actual,
    api: Object.fromEntries(Object.keys(actual.api).map((k) => [k, vi.fn()])),
  };
});

const IDLE_STATUS: CaptureStatus = {
  state: "idle",
  mode: null,
  recordingId: null,
  elapsedS: 0,
  micLevel: 0,
  systemLevel: 0,
  diskFreeMb: 20_000,
};

const TASKS = ["Accounting 302", "ENT 401"];

// RecordingRow does not carry `error` in the ipc.ts contract, but the Rust
// side is free to include it (and the brief requires failed rows to surface
// it), so RecordingList reads it defensively. The fixture models that.
const RECORDINGS: Array<RecordingRow & { error?: string | null }> = [
  {
    id: "rec-1",
    title: "Lecture 3: Depreciation",
    task: null,
    created: "2026-07-20T14:00:00Z",
    durationS: 2730,
    mode: "in_person",
    status: "ready",
    suggestedTask: "Accounting 302",
    suggestedTitle: null,
    hasNotes: false,
    error: null,
    captureNote: null,
  },
  {
    id: "rec-2",
    title: "Zoom standup",
    task: "Accounting 302",
    created: "2026-07-21T09:00:00Z",
    durationS: 1800,
    mode: "meeting",
    status: "processing",
    suggestedTask: null,
    suggestedTitle: null,
    hasNotes: false,
    error: null,
    captureNote: null,
  },
  {
    id: "rec-3",
    title: "Interrupted capture",
    task: null,
    created: "2026-07-19T08:00:00Z",
    durationS: 600,
    mode: "in_person",
    status: "failed",
    suggestedTask: null,
    suggestedTitle: null,
    hasNotes: false,
    error: "The model download was interrupted. Reconnect and reprocess.",
    captureNote: null,
  },
];

const DETAIL_REC1: RecordingDetail = {
  id: "rec-1",
  title: "Lecture 3: Depreciation",
  task: null,
  created: "2026-07-20T14:00:00Z",
  durationS: 2730,
  mode: "in_person",
  status: "ready",
  suggestedTask: "Accounting 302",
  transcriptMd:
    "[00:00:03] **Speaker 1:** Welcome back, everyone.\n[00:00:12] **George:** Let's start with depreciation.",
  summaryMd: "## TL;DR\nWe covered straight-line depreciation.",
  speakers: { spk1: "Speaker 1", george: "George" },
  notesMd: "",
  template: null,
  actions: [],
  segments: [
    { startS: 3, endS: 12, speaker: "Speaker 1", text: "Welcome back, everyone.", line: 0 },
    { startS: 12, endS: 30, speaker: "George", text: "Let's start with depreciation.", line: 1 },
  ],
  audioTracks: ["mic"],
  suggestedTitle: null,
  hasNotes: false,
  error: null,
  captureNote: null,
};

function setupApi() {
  vi.mocked(api.listTasks).mockResolvedValue(TASKS);
  vi.mocked(api.listRecordings).mockResolvedValue(RECORDINGS as RecordingRow[]);
  vi.mocked(api.getRecording).mockImplementation((id: string) => {
    if (id === "rec-1") return Promise.resolve(DETAIL_REC1);
    return Promise.reject(new Error("not found"));
  });
  vi.mocked(api.assignTask).mockResolvedValue(undefined);
  vi.mocked(api.renameRecording).mockResolvedValue(undefined);
  vi.mocked(api.renameSpeaker).mockResolvedValue(undefined);
  vi.mocked(api.updateSummary).mockResolvedValue(undefined);
  vi.mocked(api.search).mockResolvedValue([]);
  vi.mocked(api.createTask).mockResolvedValue(undefined);
  vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
  vi.mocked(api.pollMeetings).mockResolvedValue([]);
}

/**
 * Selects a recording in the list and returns its settled detail pane.
 *
 * The `await act` matters: the pane clears its drafts in a mount effect, and a
 * passive effect can still be pending when the query that waits for the pane
 * resolves. Clicking into the pane before that flush lets the reset land
 * *after* the click and silently undo it — which is what made these tests fail
 * about one run in eight.
 */
async function openRecording(title: string): Promise<HTMLElement> {
  fireEvent.click(await screen.findByText(title));
  const pane = await screen.findByRole("article", { name: "Recording" });
  await act(async () => {});
  return pane;
}

beforeEach(() => {
  vi.clearAllMocks();
  applyIpcDefaults();
  setupApi();
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("library UI", () => {
  it("renders the task list from listTasks", async () => {
    render(<App />);
    expect(await screen.findByRole("button", { name: "Accounting 302" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "ENT 401" })).toBeInTheDocument();
  });

  it("offers the suggested task on the recording, and accepting files it there", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    expect(within(pane).getByText(/looks like it belongs to/i)).toBeInTheDocument();
    fireEvent.click(within(pane).getByRole("button", { name: "File it there" }));

    await waitFor(() => expect(api.assignTask).toHaveBeenCalledWith("rec-1", "Accounting 302"));
  });

  it("hides the suggestion banner once a recording is filed under a task", async () => {
    // A recording that already has a task must not keep nagging with a
    // suggestion, even if the suggestion field is still populated.
    vi.mocked(api.listRecordings).mockResolvedValue([
      {
        id: "rec-filed",
        title: "Already filed",
        task: "Accounting 302",
        created: "2026-07-22T09:00:00Z",
        durationS: 100,
        mode: "in_person",
        status: "ready",
        suggestedTask: "Accounting 302",
        error: null,
        captureNote: null,
      },
    ] as RecordingRow[]);

    render(<App />);
    await screen.findByText("Already filed");
    expect(screen.queryByText(/Suggested:/)).not.toBeInTheDocument();
  });

  it("shows the error text on a failed row", async () => {
    render(<App />);
    expect(
      await screen.findByText("The model download was interrupted. Reconnect and reprocess.")
    ).toBeInTheDocument();
  });

  it("renames a speaker via the inline form opened by clicking their chip", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    // Radix tabs activate on mousedown, not click.
    fireEvent.mouseDown(within(pane).getByRole("tab", { name: /Transcript/ }));
    fireEvent.click(await within(pane).findByRole("button", { name: "Speaker 1" }));

    const input = screen.getByLabelText("Rename Speaker 1");
    fireEvent.change(input, { target: { value: "Jamie" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(api.renameSpeaker).toHaveBeenCalledWith("rec-1", "spk1", "Jamie")
    );
  });

  it("renames the recording when its title is edited and clicked away from", async () => {
    // Recordings are auto-titled so Record never blocks on typing a name;
    // renaming afterwards is what makes an auto-title survivable.
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    fireEvent.click(within(pane).getByRole("button", { name: "Lecture 3: Depreciation" }));

    const input = screen.getByLabelText("Recording title");
    fireEvent.change(input, { target: { value: "New name" } });
    fireEvent.blur(input);

    await waitFor(() => expect(api.renameRecording).toHaveBeenCalledWith("rec-1", "New name"));
  });

  it("saves a rename on Enter", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    fireEvent.click(within(pane).getByRole("button", { name: "Lecture 3: Depreciation" }));

    const input = screen.getByLabelText("Recording title");
    fireEvent.change(input, { target: { value: "New name" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => expect(api.renameRecording).toHaveBeenCalledTimes(1));
    expect(api.renameRecording).toHaveBeenCalledWith("rec-1", "New name");
  });

  it("cancels a rename on Escape without saving it", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    fireEvent.click(within(pane).getByRole("button", { name: "Lecture 3: Depreciation" }));

    const input = screen.getByLabelText("Recording title");
    fireEvent.change(input, { target: { value: "Typed then thought better of it" } });
    fireEvent.keyDown(input, { key: "Escape" });
    // Escape removes the input, and removing a focused element can itself fire
    // a blur — the cancel has to win that race.
    fireEvent.blur(input);

    expect(api.renameRecording).not.toHaveBeenCalled();
    expect(
      within(pane).getByRole("button", { name: "Lecture 3: Depreciation" })
    ).toBeInTheDocument();
  });

  it("does not call renameRecording when the title is left unchanged", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    fireEvent.click(within(pane).getByRole("button", { name: "Lecture 3: Depreciation" }));
    fireEvent.blur(screen.getByLabelText("Recording title"));

    expect(api.renameRecording).not.toHaveBeenCalled();
  });

  it("refuses to save a blank title", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    fireEvent.click(within(pane).getByRole("button", { name: "Lecture 3: Depreciation" }));

    const input = screen.getByLabelText("Recording title");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.blur(input);

    expect(api.renameRecording).not.toHaveBeenCalled();
  });

  it("archives a meeting and shows archived meetings in their own view", async () => {
    const archived = { ...RECORDINGS[0], archived: true } as RecordingRow;
    vi.mocked(api.listArchivedRecordings).mockResolvedValue([archived]);

    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");
    fireEvent.click(within(pane).getByRole("button", { name: "Archive" }));
    await waitFor(() => expect(api.archiveRecording).toHaveBeenCalledWith("rec-1"));
    await waitFor(() => expect(screen.queryByRole("article", { name: "Recording" })).not.toBeInTheDocument());

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    expect(await screen.findByText("Lecture 3: Depreciation")).toBeInTheDocument();
  });

  it("requires a second explicit choice before permanently deleting a meeting", async () => {
    render(<App />);
    const pane = await openRecording("Lecture 3: Depreciation");

    fireEvent.click(within(pane).getByRole("button", { name: "Delete" }));
    expect(screen.getByText(/cannot be recovered/i)).toBeInTheDocument();
    expect(api.deleteRecording).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Keep it" }));
    expect(screen.queryByText(/cannot be recovered/i)).not.toBeInTheDocument();
    expect(api.deleteRecording).not.toHaveBeenCalled();

    fireEvent.click(within(pane).getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(api.deleteRecording).toHaveBeenCalledWith("rec-1"));
  });

  it("debounces the search input before calling api.search", async () => {
    render(<App />);
    await screen.findByText("Zoom standup");

    vi.useFakeTimers();
    const search = screen.getByRole("searchbox", { name: /search/i });
    fireEvent.change(search, { target: { value: "d" } });
    fireEvent.change(search, { target: { value: "de" } });
    fireEvent.change(search, { target: { value: "dep" } });

    expect(api.search).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(500);

    expect(api.search).toHaveBeenCalledTimes(1);
    expect(api.search).toHaveBeenCalledWith("dep");
  });
});
