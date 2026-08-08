import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, within, fireEvent, waitFor, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import App from "../../App";
import { api } from "../../lib/ipc";
import { applyIpcDefaults } from "../../test/ipcMock";
import type { CaptureStatus, MeetingEvent, Settings as SettingsData } from "../../lib/ipc";

vi.mock("../../lib/ipc", async (importOriginal) => {
  // Keys derived from the real contract, so adding a command to ipc.ts can
  // never again turn every test in this file red. See src/test/ipcMock.ts.
  const actual = await importOriginal<typeof import("../../lib/ipc")>();
  return {
    ...actual,
    api: Object.fromEntries(Object.keys(actual.api).map((k) => [k, vi.fn()])),
  };
});

/**
 * The native shell, faked.
 *
 * `listen` keeps the handler it was given so a test can play the part of the
 * Rust side and fire `close-requested` or `tray-toggle-recording` itself. The
 * three destructive calls — quit, hide, unsubscribe — are recorded rather than
 * performed, because "did the app try to quit?" is the whole question in most
 * of these tests.
 *
 * `vi.hoisted` because `vi.mock` factories run before the imports above.
 */
const shell = vi.hoisted(() => {
  const handlers = new Map<string, (event: unknown) => unknown>();
  const unlisten = vi.fn();
  return {
    handlers,
    unlisten,
    listen: vi.fn(async (name: string, handler: (event: unknown) => unknown) => {
      handlers.set(name, handler);
      return unlisten;
    }),
    exit: vi.fn(async () => {}),
    hide: vi.fn(async () => {}),
    // The custom titlebar's controls read this on mount and follow it after.
    // Stubbed rather than left off so the desktop tests below render the real
    // header instead of one whose maximise button quietly failed to set up.
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
    register: vi.fn(async (_shortcut: string, _handler: unknown) => {}),
    unregister: vi.fn(async (_shortcuts: string | string[]) => {}),
    unregisterAll: vi.fn(async () => {}),
    isEnabled: vi.fn(async () => false),
    enable: vi.fn(async () => {}),
    disable: vi.fn(async () => {}),
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: shell.listen,
  emit: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    hide: shell.hide,
    isMaximized: shell.isMaximized,
    onResized: shell.onResized,
  }),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ exit: shell.exit }));
// The OS-wide hotkeys: App registers them the moment settings settle. Stubbed
// rather than left real so these tests never depend on a global-shortcut
// round trip they have nothing to say about — and so "which accelerator is
// live right now" is something a test can actually read.
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: shell.register,
  unregister: shell.unregister,
  unregisterAll: shell.unregisterAll,
}));
// Same for the login item, which App writes once on a fresh install.
vi.mock("@tauri-apps/plugin-autostart", () => ({
  isEnabled: shell.isEnabled,
  enable: shell.enable,
  disable: shell.disable,
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
  convertFileSrc: (path: string) => path,
  // The auto-updater asks this first. False keeps a 30-second timer from ever
  // becoming this file's problem.
  isTauri: () => false,
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

const BASE_SETTINGS: SettingsData = {
  overlay: "recording",
  hotkeyHighlight: "CommandOrControl+Alt+H",
  storageRoot: "/Users/george/Notetaker",
  llmBaseUrl: "http://localhost:11434",
  llmModel: "qwen2.5:7b",
  taskModels: {},
  templates: [],
  summaryPrompt: "",
  tierOverride: null,
  processWhenIdle: true,
  autoRecord: {},
  minIdleSecs: 300,
  requireAc: true,
  keepWav: false,
  languages: ["en"],
  speechEngine: "auto",
  inputDevice: null,
  hotkeyToggleRecord: "CommandOrControl+Alt+N",
  hotkeyShowHide: "CommandOrControl+Alt+Space",
  closeToTray: true,
  audioDevicePriority: [],
  performanceMode: "auto",
  modelIdleUnload: "5m",
  cleanupModel: "llama3.2:3b",
  dictationCleanupEnabled: true,
  dictationDictionary: [],
  dictationReplacements: {},
  dictationMode: "pushToTalk",
  dictationPasteBehavior: "paste",
  dictationHotkey: "CommandOrControl+Alt+D",
  dictationKeepAudio: false,
  overlayPosition: "topRight",
  overlayStyle: "glass",
  overlayHideFromShare: true,
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
  applyIpcDefaults();
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

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.any(String))
    );
  });

  it("picking In-person then Start calls startCapture with mode 'in_person'", async () => {
    vi.mocked(api.startCapture).mockResolvedValue({ ...RECORDING_STATUS, systemLevel: 0 });
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Choose what to record" }));
    fireEvent.click(await screen.findByRole("button", { name: /In person/ }));

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("in_person", expect.any(String))
    );
  });

  it("pauses a recording (flipping the button to Resume) then resumes it", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.pauseCapture).mockResolvedValue(PAUSED_STATUS);
    vi.mocked(api.resumeCapture).mockResolvedValue(RECORDING_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));
    const pauseButton = await screen.findByRole("button", { name: "Pause" });
    await waitFor(() => expect(pauseButton).toBeEnabled());

    fireEvent.click(pauseButton);
    await waitFor(() => expect(api.pauseCapture).toHaveBeenCalledTimes(1));

    const resumeButton = await screen.findByRole("button", { name: "Resume" });
    fireEvent.click(resumeButton);
    await waitFor(() => expect(api.resumeCapture).toHaveBeenCalledTimes(1));
  });

  it("Stop calls stopCapture and the control returns to idle", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.stopCapture).mockResolvedValue("rec-live");
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));
    const stopButton = await screen.findByRole("button", { name: "Stop" });
    await waitFor(() => expect(stopButton).toBeEnabled());

    fireEvent.click(stopButton);
    await waitFor(() => expect(api.stopCapture).toHaveBeenCalledTimes(1));

    // Back to idle: the record control returns and the live controls are gone.
    await waitFor(() => expect(screen.getByRole("button", { name: "Record" })).toBeEnabled());
    expect(screen.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });

  /**
   * The control swaps rather than greying out: idle shows Record, a live
   * session shows Pause and Stop. What has to hold either way is that no state
   * ever offers an action it cannot perform — a Pause with nothing to pause, or
   * a second Record over a running session.
   */
  it("offers only the actions that are legal for the current capture state", async () => {
    render(<App />);

    expect(await screen.findByRole("button", { name: "Record" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();

    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    fireEvent.click(screen.getByRole("button", { name: "Record" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled()
    );
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Record" }),
      "a second recording must not be startable over a running one"
    ).not.toBeInTheDocument();
  });

  it("says it is still saving, and refuses a new recording, until the last one lands", async () => {
    // The auto-stop path — a full disk or a dead mic ends the recording with
    // nobody pressing Stop, so this snapshot is the only thing the UI is told.
    vi.mocked(api.captureStatus).mockResolvedValue(FINISHING_STATUS);
    render(<App />);

    expect(await screen.findByText(/saving your recording/i)).toBeInTheDocument();
    // Record is present but refused: the last recording has not landed, and
    // starting another here would look like the first never happened.
    expect(screen.getByRole("button", { name: "Record" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Choose what to record" })).toBeDisabled();
  });

  it("follows a self-stopping recording through saving and only then re-arms Record", async () => {
    // Real timers: this is about the status poll, and the poll is the only
    // thing that tells the UI a self-stopping recording ended.
    const patience = { timeout: 4000 };
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled());

    // The session ends itself — a full disk, a dead mic — and nobody presses
    // Stop, so the next poll is the first the UI hears of it.
    vi.mocked(api.captureStatus).mockResolvedValue(FINISHING_STATUS);
    await waitFor(
      () => expect(screen.getByText(/saving your recording/i)).toBeInTheDocument(),
      patience
    );
    expect(screen.getByRole("button", { name: "Record" })).toBeDisabled();

    vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
    await waitFor(
      () => expect(screen.getByRole("button", { name: "Record" })).toBeEnabled(),
      patience
    );
    expect(screen.queryByText(/saving your recording/i)).not.toBeInTheDocument();
  });

  /**
   * The meters appear only while a session is live — a meter pinned at zero
   * because nothing is being captured reads as "your microphone is dead".
   * Which meters appear follows the *running session's* mode, not the picker's.
   */
  it("shows the system level meter in meeting mode and hides it in in-person mode", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    const { unmount } = render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));
    expect(
      await screen.findByRole("progressbar", { name: "Microphone level" })
    ).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "System audio level" })).toBeInTheDocument();
    unmount();

    vi.mocked(api.startCapture).mockResolvedValue({ ...RECORDING_STATUS, mode: "in_person" });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Choose what to record" }));
    fireEvent.click(await screen.findByRole("button", { name: /In person/ }));

    expect(
      await screen.findByRole("progressbar", { name: "Microphone level" })
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("progressbar", { name: "System audio level" }),
      "an in-person recording has no system track, so it must not claim a level for one"
    ).not.toBeInTheDocument();
  });

  it("refreshes the live meters independently of the one-second capture-status poll", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    vi.mocked(api.captureLevels).mockResolvedValue({ micLevel: 0.86, systemLevel: 0.42 });
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Record" }));

    const mic = await screen.findByRole("progressbar", { name: "Microphone level" });
    await waitFor(() => expect(mic).toHaveAttribute("aria-valuenow", "86"));
    expect(api.captureLevels).toHaveBeenCalledTimes(1);

    await waitFor(() => expect(vi.mocked(api.captureLevels).mock.calls.length).toBeGreaterThan(1), {
      timeout: 500,
    });
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
    await screen.findByRole("button", { name: "Record" });
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

    fireEvent.click(await screen.findByRole("button", { name: "Record this meeting" }));

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

    await screen.findByRole("button", { name: "Record" });
    // Give any (incorrect) async start a chance to have fired.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(api.startCapture).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });
});

/**
 * The desktop shell's half of closing, the tray menu, the OS-wide hotkeys, and
 * the login item.
 *
 * None of this exists in a browser: the effect that registers these listeners
 * returns immediately when `isDesktop()` is false, which is why every other
 * test in this file never sees them. Here `__TAURI_INTERNALS__` is stubbed onto
 * `window` — the same trick `transport.test.ts` uses — and removed again
 * afterwards so desktop mode never leaks into a neighbouring test.
 */
describe("closing the window, the tray menu, and the OS-wide shell", () => {
  beforeEach(() => {
    shell.handlers.clear();
    window.localStorage.clear();
    // The first-run card is a modal of its own; dismissing it up front keeps
    // it from sitting on top of the dialogs under test.
    window.localStorage.setItem("notetaker.firstRunDismissed", "1");
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    window.localStorage.clear();
  });

  /** Renders, then waits until the listeners are up and the settings landed. */
  async function mount(settings: Partial<SettingsData> = {}) {
    vi.mocked(api.getSettings).mockResolvedValue({ ...BASE_SETTINGS, ...settings });
    const view = render(<App />);
    // The sidebar prints the record hotkey only once getSettings has resolved
    // *and* we are on the desktop — precisely the state `closeToTrayRef` reads
    // from, so this is the honest signal that the listeners can be fired.
    await screen.findByText(/hit record, or press/i);
    await waitFor(() => {
      expect([...shell.handlers.keys()]).toEqual(
        expect.arrayContaining([
          "close-requested",
          "tray-toggle-recording",
          "tray-record",
          "tray-pause-resume",
          "tray-stop",
          "overlay-record",
          "overlay-dismiss",
          "overlay-highlight",
          "overlay-pause-resume",
          "overlay-stop",
          "overlay-open",
          "tray-open-settings",
          "tray-quit-requested",
        ]),
      );
    });
    return view;
  }

  /** Plays the part of the Rust side emitting one of its events. */
  async function emit(name: string) {
    const handler = shell.handlers.get(name);
    if (!handler) throw new Error(`nothing is listening for "${name}"`);
    await act(async () => {
      await handler({ event: name, id: 1, payload: null });
    });
  }

  it("offers to save first when quitting would end a live recording", async () => {
    vi.mocked(api.captureStatus).mockResolvedValue(RECORDING_STATUS);
    await mount({ closeToTray: false });

    await emit("close-requested");

    expect(await screen.findByText("Recording in progress")).toBeInTheDocument();
    expect(shell.exit).not.toHaveBeenCalled();
  });

  it("quits on close when close-to-tray is off and nothing is recording", async () => {
    await mount({ closeToTray: false });

    await emit("close-requested");

    await waitFor(() => expect(shell.exit).toHaveBeenCalledWith(0));
  });

  it("explains the tray the first time rather than appearing to vanish", async () => {
    await mount({ closeToTray: true });

    await emit("close-requested");

    expect(await screen.findByText("Still running")).toBeInTheDocument();
    expect(shell.hide).not.toHaveBeenCalled();
    expect(shell.exit).not.toHaveBeenCalled();
  });

  it("hides without a word once that note has been read", async () => {
    window.localStorage.setItem("notetaker.trayExplained", "1");
    await mount({ closeToTray: true });

    await emit("close-requested");

    await waitFor(() => expect(shell.hide).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("Still running")).not.toBeInTheDocument();
    expect(shell.exit).not.toHaveBeenCalled();
  });

  /**
   * The hole this test exists for. On the first close of a fresh install the
   * tray note is shown *before* anything has looked at the recording, so its
   * "Quit instead" button is a second, later route to quitting — and it used
   * to take it unconditionally, ending a live take that nothing had saved.
   */
  it("will not let the tray note's Quit instead end a live recording", async () => {
    vi.mocked(api.captureStatus).mockResolvedValue(RECORDING_STATUS);
    await mount({ closeToTray: true });

    await emit("close-requested");
    expect(await screen.findByText("Still running")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Quit instead" }));

    expect(await screen.findByText("Recording in progress")).toBeInTheDocument();
    expect(shell.exit).not.toHaveBeenCalled();
  });

  it("the tray's toggle starts a meeting recording when idle", async () => {
    vi.mocked(api.startCapture).mockResolvedValue(RECORDING_STATUS);
    await mount();

    await emit("tray-toggle-recording");

    await waitFor(() =>
      expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.any(String))
    );
  });

  it("the tray's toggle stops a running recording", async () => {
    vi.mocked(api.captureStatus).mockResolvedValue(RECORDING_STATUS);
    await mount();

    await emit("tray-toggle-recording");

    await waitFor(() => expect(api.stopCapture).toHaveBeenCalledTimes(1));
    expect(api.startCapture).not.toHaveBeenCalled();
  });

  it("the tray's toggle does nothing while a recording is still saving", async () => {
    // `finishing` is neither startable nor stoppable: pressing it would either
    // lose the take that is landing or start a second one on top of it.
    vi.mocked(api.captureStatus).mockResolvedValue(FINISHING_STATUS);
    await mount();

    await emit("tray-toggle-recording");

    expect(api.startCapture).not.toHaveBeenCalled();
    expect(api.stopCapture).not.toHaveBeenCalled();
  });

  /**
   * The tray's Quit used to be `app.exit(0)` in the Rust menu handler, which
   * bypasses the webview entirely: it ended a live take mid-buffer and left it
   * to be recovered as a crash on the next launch. It now emits and asks, so
   * these two prove both halves of the answer.
   */
  it("the tray's Quit offers to save first when a recording is live", async () => {
    vi.mocked(api.captureStatus).mockResolvedValue(RECORDING_STATUS);
    await mount();

    await emit("tray-quit-requested");

    expect(await screen.findByText("Recording in progress")).toBeInTheDocument();
    expect(shell.exit).not.toHaveBeenCalled();
  });

  it("the tray's Quit exits straight away when nothing is recording", async () => {
    await mount();

    await emit("tray-quit-requested");

    await waitFor(() => expect(shell.exit).toHaveBeenCalledWith(0));
    expect(screen.queryByText("Recording in progress")).not.toBeInTheDocument();
  });

  /**
   * A rebind has to reach the OS *now*, not when the panel is dismissed.
   *
   * `appSettings` used to refetch only when `settingsOpen` flipped, so between
   * pressing the new chord and closing Settings the OLD accelerator was still
   * the live OS shortcut and the new one did nothing. And because
   * `hotkeys.issues` is filled in by the registration, a combination already
   * taken by another app said so only on the *next* open of Settings — silent
   * at exactly the moment the mistake was made.
   */
  it("re-registers a rebound hotkey without waiting for Settings to close", async () => {
    // Persist for real, so App's refetch can see the new value. Without this
    // every later getSettings would answer with the accelerator from before.
    vi.mocked(api.setSettings).mockImplementation(async (next) => {
      vi.mocked(api.getSettings).mockResolvedValue(next);
    });
    await mount();
    await waitFor(() =>
      expect(shell.register.mock.calls.map((c) => c[0])).toContain("CommandOrControl+Alt+N")
    );

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const dialog = await screen.findByRole("dialog", { name: "Settings" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Shortcuts" }));
    const field = await within(dialog).findByRole("button", {
      name: "Change shortcut: Start / stop recording",
    });
    fireEvent.click(field);
    fireEvent.keyDown(field, { key: "k", code: "KeyK", ctrlKey: true, altKey: true });

    await waitFor(() =>
      expect(shell.register.mock.calls.map((c) => c[0])).toContain("CommandOrControl+Alt+K")
    );
    // The panel is still open. That is the whole point.
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
  });

  it("turns start-with-Windows on once, and never asks again", async () => {
    await mount();

    await waitFor(() => expect(shell.enable).toHaveBeenCalledTimes(1));
    expect(window.localStorage.getItem("notetaker.autostartInit")).toBe("1");

    cleanup();
    await mount();
    // Still once: the marker is what stops a second launch from quietly
    // undoing someone who turned it off in Settings.
    expect(shell.enable).toHaveBeenCalledTimes(1);
  });

  it("asks again next launch when the OS refuses start-with-Windows", async () => {
    shell.enable.mockRejectedValueOnce(new Error("no permission"));
    await mount();

    await waitFor(() => expect(shell.enable).toHaveBeenCalledTimes(1));
    // Not promoted to done. Marking it done on a refusal would leave autostart
    // off forever with the app believing it had set it.
    expect(window.localStorage.getItem("notetaker.autostartInit")).not.toBe("1");

    cleanup();
    await mount();
    await waitFor(() => expect(shell.enable).toHaveBeenCalledTimes(2));
  });

  /**
   * The other half of that trade, and the one that bites hardest.
   *
   * The marker is what stops the app asking twice. If the store cannot keep it,
   * asking anyway would re-enable a login item somebody deliberately turned off
   * in Settings — on every single launch, with no way to make it stick. So the
   * first write doubles as a writability probe and has to happen *before* the
   * OS is touched at all.
   */
  it("never touches the OS when the store cannot remember the decision", async () => {
    const realSetItem = Storage.prototype.setItem;
    const setItem = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(function (this: Storage, key: string, value: string) {
        // Only this key, so nothing unrelated (the theme, the first-run card)
        // starts failing and turns this into a test about something else.
        if (key === "notetaker.autostartInit") throw new Error("QuotaExceededError");
        realSetItem.call(this, key, value);
      });

    try {
      await mount();
      // Bounded, and generous: the effect fires on mount, so anything it was
      // going to do has happened long before this resolves.
      await new Promise((resolve) => setTimeout(resolve, 50));

      expect(shell.enable).not.toHaveBeenCalled();
      expect(window.localStorage.getItem("notetaker.autostartInit")).toBeNull();
    } finally {
      setItem.mockRestore();
    }
  });

  it("drops its subscriptions when the shell goes away", async () => {
    const { unmount } = await mount();
    const before = shell.unlisten.mock.calls.length;
    const subscriptions = shell.handlers.size;

    unmount();

    await waitFor(() => expect(shell.unlisten.mock.calls.length).toBe(before + subscriptions));
  });
});
