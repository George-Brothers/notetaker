import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, within, fireEvent, waitFor, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import App from "../../App";
import { api } from "../../lib/ipc";
import type { RecordingDetail, RecordingRow } from "../../lib/ipc";

vi.mock("../../lib/ipc", () => ({
  api: {
    listTasks: vi.fn(),
    createTask: vi.fn(),
    listRecordings: vi.fn(),
    getRecording: vi.fn(),
    search: vi.fn(),
    processNow: vi.fn(),
    assignTask: vi.fn(),
    renameSpeaker: vi.fn(),
    getSettings: vi.fn(),
    setSettings: vi.fn(),
  },
}));

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
    error: null,
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
    error: null,
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
    error: "The model download was interrupted. Reconnect and reprocess.",
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
  error: null,
};

function setupApi() {
  vi.mocked(api.listTasks).mockResolvedValue(TASKS);
  vi.mocked(api.listRecordings).mockResolvedValue(RECORDINGS as RecordingRow[]);
  vi.mocked(api.getRecording).mockImplementation((id: string) => {
    if (id === "rec-1") return Promise.resolve(DETAIL_REC1);
    return Promise.reject(new Error("not found"));
  });
  vi.mocked(api.assignTask).mockResolvedValue(undefined);
  vi.mocked(api.renameSpeaker).mockResolvedValue(undefined);
  vi.mocked(api.search).mockResolvedValue([]);
  vi.mocked(api.createTask).mockResolvedValue(undefined);
}

beforeEach(() => {
  vi.clearAllMocks();
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

  it("shows a suggested task with Accept, and accepting assigns it", async () => {
    render(<App />);
    const titleNode = await screen.findByText("Lecture 3: Depreciation");
    const row = titleNode.closest("li") as HTMLElement;
    expect(within(row).getByText("Suggested: Accounting 302")).toBeInTheDocument();

    fireEvent.click(within(row).getByRole("button", { name: "Accept" }));

    await waitFor(() => expect(api.assignTask).toHaveBeenCalledWith("rec-1", "Accounting 302"));
  });

  it("shows the error text on a failed row", async () => {
    render(<App />);
    expect(
      await screen.findByText("The model download was interrupted. Reconnect and reprocess.")
    ).toBeInTheDocument();
  });

  it("renames a speaker via the inline form opened by clicking their name", async () => {
    render(<App />);
    fireEvent.click(await screen.findByText("Lecture 3: Depreciation"));

    const speakerButton = await screen.findByRole("button", { name: "Speaker 1" });
    fireEvent.click(speakerButton);

    const input = screen.getByLabelText("Rename Speaker 1");
    fireEvent.change(input, { target: { value: "Jamie" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(api.renameSpeaker).toHaveBeenCalledWith("rec-1", "spk1", "Jamie")
    );
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
