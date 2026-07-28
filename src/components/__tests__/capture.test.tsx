import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import App from "../../App";
import { api } from "../../lib/ipc";
import type { CaptureStatus, MeetingEvent } from "../../lib/ipc";

vi.mock("../../lib/ipc", () => ({
  api: {
    listTasks: vi.fn(),
    createTask: vi.fn(),
    listRecordings: vi.fn(),
    getRecording: vi.fn(),
    search: vi.fn(),
    processNow: vi.fn(),
    updateSummary: vi.fn(),
    assignTask: vi.fn(),
    renameSpeaker: vi.fn(),
    getSettings: vi.fn(),
    setSettings: vi.fn(),
    startCapture: vi.fn(),
    pauseCapture: vi.fn(),
    resumeCapture: vi.fn(),
    stopCapture: vi.fn(),
    captureStatus: vi.fn(),
    pollMeetings: vi.fn(),
    setAutoRecord: vi.fn(),
  },
}));

const IDLE_STATUS: CaptureStatus = {
  state: "idle",
  mode: null,
  recordingId: null,
  elapsedS: 0,
  micLevel: 0,
  systemLevel: 0,
  diskFreeMb: 20_000,
};

const RECORDING_STATUS: CaptureStatus = {
  state: "recording",
  mode: "meeting",
  recordingId: "rec-live",
  elapsedS: 5,
  micLevel: 0.4,
  systemLevel: 0.2,
  diskFreeMb: 20_000,
};

const PAUSED_STATUS: CaptureStatus = {
  ...RECORDING_STATUS,
  state: "paused",
};

/**
 * Capture is over but the recording is still being encoded and queued. The
 * meters read flat and there is no mode, because nothing is being captured.
 */
const FINISHING_STATUS: CaptureStatus = {
  state: "finishing",
  mode: null,
  recordingId: "rec-live",
  elapsedS: 5,
  micLevel: 0,
  systemLevel: 0,
  diskFreeMb: 20_000,
};

const ZOOM_STARTED_ASK: MeetingEvent = {
  appId: "zoom",
  appName: "Zoom",
  kind: "started",
  autoStart: false,
};

const ZOOM_STARTED_ALWAYS: MeetingEvent = {
  appId: "zoom",
  appName: "Zoom",
  kind: "started",
  autoStart: true,
};

function setupApi() {
  vi.mocked(api.listTasks).mockResolvedValue([]);
  vi.mocked(api.listRecordings).mockResolvedValue([]);
  vi.mocked(api.search).mockResolvedValue([]);
  vi.mocked(api.createTask).mockResolvedValue(undefined);
  vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
  vi.mocked(api.pollMeetings).mockResolvedValue([]);
  vi.mocked(api.setAutoRecord).mockResolvedValue(undefined);
}

beforeEach(() => {
  vi.clearAllMocks();
  setupApi();
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("record bar", () => {
  it("defaults to meeting mode: Start calls startCapture with mode 'meeting'", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Start" }));

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.any(String))
    );
  });

  it("picking In-person then Start calls startCapture with mode 'in_person'", async () => {
    vi.mocked(api.startCapture).mockResolvedValue({ ...RECORDING_STATUS, systemLevel: 0 });
    render(<App />);

    fireEvent.click(await screen.findByRole("radio", { name: "In-person" }));
    fireEvent.click(screen.getByRole("button", { name: "Start" }));

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("in_person", expect.any(String))
    );
  });

  it("pauses a recording (flipping the button to Resume) then resumes it", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.pauseCapture).mockResolvedValue(PAUSED_STATUS);
    vi.mocked(api.resumeCapture).mockResolvedValue(RECORDING_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Start" }));
    const pauseButton = await screen.findByRole("button", { name: "Pause" });
    await waitFor(() => expect(pauseButton).toBeEnabled());

    fireEvent.click(pauseButton);
    await waitFor(() => expect(api.pauseCapture).toHaveBeenCalledTimes(1));

    const resumeButton = await screen.findByRole("button", { name: "Resume" });
    fireEvent.click(resumeButton);
    await waitFor(() => expect(api.resumeCapture).toHaveBeenCalledTimes(1));
  });

  it("Stop calls stopCapture and the bar returns to idle", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.stopCapture).mockResolvedValue("rec-live");
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Start" }));
    const stopButton = await screen.findByRole("button", { name: "Stop" });
    await waitFor(() => expect(stopButton).toBeEnabled());

    fireEvent.click(stopButton);
    await waitFor(() => expect(api.stopCapture).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByRole("button", { name: "Start" })).toBeEnabled());
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
  });

  it("enables only the legal actions for the current capture state", async () => {
    render(<App />);

    expect(await screen.findByRole("button", { name: "Start" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();

    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    fireEvent.click(screen.getByRole("button", { name: "Start" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Start" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();
  });

  it("says it is still saving, and refuses a new recording, until the last one lands", async () => {
    // The auto-stop path — a full disk or a dead mic ends the recording with
    // nobody pressing Stop, so this snapshot is the only thing the UI is told.
    vi.mocked(api.captureStatus).mockResolvedValue(FINISHING_STATUS);
    render(<App />);

    expect(await screen.findByText(/saving your recording/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    // Stop stays live: a save that failed is retried by pressing it again.
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();
  });

  it("follows a self-stopping recording through saving and only then re-arms Start", async () => {
    // Real timers: this is about the status poll, and the poll is the only
    // thing that tells the UI a self-stopping recording ended.
    const patience = { timeout: 4000 };
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Start" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Start" })).toBeDisabled());

    // The session ends itself — a full disk, a dead mic — and nobody presses
    // Stop, so the next poll is the first the UI hears of it.
    vi.mocked(api.captureStatus).mockResolvedValue(FINISHING_STATUS);
    await waitFor(
      () => expect(screen.getByText(/saving your recording/i)).toBeInTheDocument(),
      patience
    );
    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();

    vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
    await waitFor(
      () => expect(screen.getByRole("button", { name: "Start" })).toBeEnabled(),
      patience
    );
    expect(screen.queryByText(/saving your recording/i)).not.toBeInTheDocument();
  });

  it("shows the system level meter in meeting mode and hides it in in-person mode", async () => {
    render(<App />);
    await screen.findByRole("button", { name: "Start" });

    expect(screen.getByRole("progressbar", { name: "System audio level" })).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Microphone level" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("radio", { name: "In-person" }));

    expect(screen.queryByRole("progressbar", { name: "System audio level" })).not.toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Microphone level" })).toBeInTheDocument();
  });

  it("shows a low-disk warning in plain language when free space is low", async () => {
    vi.mocked(api.captureStatus).mockResolvedValue({ ...IDLE_STATUS, diskFreeMb: 400 });
    render(<App />);

    expect(
      await screen.findByText(/low on disk space/i)
    ).toBeInTheDocument();
  });

  it("does not show the low-disk warning when free space is plentiful", async () => {
    render(<App />);
    await screen.findByRole("button", { name: "Start" });
    expect(screen.queryByText(/low on disk space/i)).not.toBeInTheDocument();
  });
});

describe("meeting-detected prompt", () => {
  it("prompts to record when a meeting app starts under the ask policy", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ASK]);
    render(<App />);

    expect(await screen.findByText("Zoom started — record this?")).toBeInTheDocument();
  });

  it("Record starts a meeting recording and closes the prompt", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ASK]);
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.stringContaining("Zoom"))
    );
    expect(screen.queryByText("Zoom started — record this?")).not.toBeInTheDocument();
  });

  it("Not now dismisses the prompt without recording", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ASK]);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Not now" }));

    expect(api.startCapture).not.toHaveBeenCalled();
    expect(screen.queryByText("Zoom started — record this?")).not.toBeInTheDocument();
  });

  it("Always record Zoom persists the policy and starts recording immediately", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ASK]);
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Always record Zoom" }));

    await waitFor(() => expect(api.setAutoRecord).toHaveBeenCalledWith("zoom", "always"));
    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.stringContaining("Zoom"))
    );
  });

  it("Never ask for Zoom persists the policy without starting a recording", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ASK]);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Never ask for Zoom" }));

    await waitFor(() => expect(api.setAutoRecord).toHaveBeenCalledWith("zoom", "never"));
    expect(api.startCapture).not.toHaveBeenCalled();
  });

  it("Escape behaves as Not now", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ASK]);
    render(<App />);

    const dialog = await screen.findByRole("alertdialog");
    fireEvent.keyDown(dialog, { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(api.startCapture).not.toHaveBeenCalled();
  });

  it("an autoStart:true started event records with no prompt at all", async () => {
    vi.mocked(api.pollMeetings).mockResolvedValue([ZOOM_STARTED_ALWAYS]);
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    render(<App />);

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.any(String))
    );
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("an 'ended' event with autoStart:true (an always-policy app closing) starts nothing and prompts nothing", async () => {
    // Sharp edge from the watcher: `autoStart` reflects the app's policy,
    // not the event kind, so an "always" policy app also stamps autoStart
    // on its "ended" event. Keying off autoStart alone would wrongly start
    // a new recording the moment the meeting ends.
    vi.mocked(api.pollMeetings).mockResolvedValue([
      { appId: "zoom", appName: "Zoom", kind: "ended", autoStart: true },
    ]);
    render(<App />);

    await screen.findByRole("button", { name: "Start" });
    // Give any (incorrect) async start a chance to have fired.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(api.startCapture).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });
});
