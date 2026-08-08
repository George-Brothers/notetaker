import { useState } from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, within, fireEvent, waitFor, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import App from "../../App";
import { Settings as SettingsComponent } from "../../components/Settings";
import type { SettingsProps, SettingsSection } from "../../components/Settings";
import { useTheme } from "../../hooks/useTheme";
import { api } from "../../lib/ipc";
import { applyIpcDefaults } from "../../test/ipcMock";
import type { CaptureStatus, FoundModel, OllamaStatus, PullProgress, Settings, SetupStatus } from "../../lib/ipc";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { checkForUpdate, installUpdate } from "../../lib/updater";
import { getAutostart, pickFolder, setAutostart } from "../../lib/desktop";

vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("../../lib/updater", () => ({ checkForUpdate: vi.fn(), installUpdate: vi.fn() }));

// Only the three plugin-backed shell services are faked. `listInputDevices`
// and `trayStateFor` stay real, because the microphone list is part of what
// the Recording section renders and it already answers safely off the desktop.
vi.mock("../../lib/desktop", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/desktop")>()),
  pickFolder: vi.fn(),
  getAutostart: vi.fn(),
  setAutostart: vi.fn(),
}));

vi.mock("../../lib/ipc", async (importOriginal) => {
  // Keys derived from the real contract, so adding a command to ipc.ts can
  // never again turn every test in this file red. See src/test/ipcMock.ts.
  const actual = await importOriginal<typeof import("../../lib/ipc")>();
  return {
    ...actual,
    api: Object.fromEntries(Object.keys(actual.api).map((k) => [k, vi.fn()])),
  };
});

// jsdom has no ResizeObserver; cmdk's Command.List uses one purely to track
// its own rendered height for a CSS variable. The "palette deep link"
// App-integration test below is the only test in this file that opens the
// command palette (see commandPalette.test.tsx for the same stub, needed for
// the same reason).
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

// jsdom also has no scrollIntoView; cmdk calls it once on mount to scroll the
// initially-selected row into view. Layout-only, so a no-op is enough here too.
Element.prototype.scrollIntoView = vi.fn();

const IDLE_STATUS: CaptureStatus = {
  state: "idle",
  mode: null,
  recordingId: null,
  elapsedS: 0,
  micLevel: 0,
  systemLevel: 0,
  diskFreeMb: 20_000,
};

const BASE_SETTINGS: Settings = {
  overlay: "recording",
  hotkeyHighlight: "CommandOrControl+Alt+H",
  storageRoot: "/Users/george/Notetaker",
  llmBaseUrl: "http://localhost:11434",
  llmModel: "qwen2.5:7b",
  taskModels: {},
  templates: [],
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

const OLLAMA_NOT_INSTALLED: OllamaStatus = {
  installed: false,
  running: false,
  models: [],
  modelReady: false,
  installHint: "Install Ollama from ollama.com, then reopen Notetaker.",
};

const OLLAMA_NEEDS_MODEL: OllamaStatus = {
  installed: true,
  running: true,
  models: [],
  modelReady: false,
  installHint: null,
};

const OLLAMA_STOPPED: OllamaStatus = {
  installed: true,
  running: false,
  models: [],
  modelReady: false,
  installHint: "Ollama is installed but not running. Open it and summaries will start working — nothing else needs setting up.",
};

const OLLAMA_READY: OllamaStatus = {
  installed: true,
  running: true,
  models: ["qwen2.5:7b"],
  modelReady: true,
  installHint: null,
};

const SETUP_WITHOUT_FOUND_MODELS: SetupStatus = {
  transcribing: false,
  missing: [{ name: "whisper-small-q5_1", label: "Speech model (fast)", bytes: 190085487 }],
  downloadBytes: 190085487,
  waiting: 0,
  tier: "CpuSmall",
};

function setupApi(overrides: { settings?: Settings; ollama?: OllamaStatus; found?: FoundModel[]; setup?: SetupStatus; tasks?: string[] } = {}) {
  vi.mocked(api.listTasks).mockResolvedValue(overrides.tasks ?? []);
  vi.mocked(api.listRecordings).mockResolvedValue([]);
  vi.mocked(api.listArchivedRecordings).mockResolvedValue([]);
  vi.mocked(api.search).mockResolvedValue([]);
  vi.mocked(api.createTask).mockResolvedValue(undefined);
  vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
  vi.mocked(api.pollMeetings).mockResolvedValue([]);
  vi.mocked(api.setAutoRecord).mockResolvedValue(undefined);
  vi.mocked(api.getSettings).mockResolvedValue(overrides.settings ?? BASE_SETTINGS);
  vi.mocked(api.setSettings).mockResolvedValue(undefined);
  vi.mocked(api.detectedTier).mockResolvedValue("medium");
  vi.mocked(api.ollamaStatus).mockResolvedValue(overrides.ollama ?? OLLAMA_NEEDS_MODEL);
  vi.mocked(api.pullProgress).mockResolvedValue([]);
  vi.mocked(api.pullModel).mockResolvedValue(undefined);
  vi.mocked(api.downloadModels).mockResolvedValue(undefined);
  vi.mocked(api.adoptModels).mockResolvedValue(undefined);
  vi.mocked(api.findExistingModels).mockResolvedValue(overrides.found ?? []);
  vi.mocked(api.setupStatus).mockResolvedValue(overrides.setup ?? SETUP_WITHOUT_FOUND_MODELS);
  vi.mocked(api.logPath).mockResolvedValue("/Users/george/Library/Notetaker/logs/notetaker.log");
}

/**
 * Renders `<Settings>` directly rather than through `<App>`.
 *
 * The old helper opened Settings via the header's gear button and never had
 * to address a specific section — every control lived on one long scroll.
 * `Settings` now takes `section`/`onSelectSection` as a controlled pair
 * instead of a mount-only `initialSection` (Task 6 review fix — App is the
 * single source of truth so a palette deep link can retarget an
 * already-open panel), so this host owns that piece of state itself and
 * exposes it to callers under the old `initialSection` name, since that's
 * what every test below already calls it. `onClose` still does a real
 * unmount (not a `vi.fn()` stub) so "can be closed with the close button"
 * keeps testing real close behavior rather than a mock that was merely
 * called.
 */
type SettingsHostProps = Partial<Omit<SettingsProps, "section" | "onSelectSection">> & {
  initialSection?: SettingsSection;
};

function SettingsHost({ initialSection, ...props }: SettingsHostProps) {
  const theme = useTheme();
  const [open, setOpen] = useState(true);
  const [section, setSection] = useState<SettingsSection>(initialSection ?? "general");
  if (!open) return null;
  return (
    <SettingsComponent
      onClose={() => setOpen(false)}
      theme={theme}
      section={section}
      onSelectSection={setSection}
      {...props}
    />
  );
}

const SECTION_HEADINGS: Record<SettingsSection, string> = {
  general: "General",
  shortcuts: "Shortcuts",
  audio: "Audio",
  models: "Models & AI",
  dictation: "Dictation",
  overlay: "Overlay",
  meetings: "Meetings",
  storage: "Storage & Privacy",
  updates: "Updates",
};

async function openSettings(props: SettingsHostProps = {}) {
  render(<SettingsHost {...props} />);
  const dialog = await screen.findByRole("dialog", { name: "Settings" });
  // Wait for the requested section (General by default) to be the one
  // actually showing before handing the dialog back to the test. For every
  // section except "updates" this already implies settings finished
  // loading, since those panes are gated behind `!settings`.
  await within(dialog).findByRole("heading", { name: SECTION_HEADINGS[props.initialSection ?? "general"] });
  // "updates" renders independent of that gate by design (checking for an
  // app update shouldn't depend on Settings data loading first), so its
  // heading can appear before getSettings/detectedTier/ollamaStatus/
  // setupStatus/listInputDevices have resolved. Flush them here, inside
  // `act`, so every call site gets fully-settled state back — otherwise
  // those four background updates land after the test (or `cleanup()`) has
  // already moved on, which is what an "act" warning is warning about.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  return dialog;
}

/** The most recent settings object written through `api.setSettings`. */
function lastSetSettings(): unknown {
  const calls = vi.mocked(api.setSettings).mock.calls;
  return calls[calls.length - 1]?.[0];
}

beforeEach(() => {
  vi.clearAllMocks();
  applyIpcDefaults();
  localStorage.clear();
  setupApi();
  vi.mocked(checkForUpdate).mockResolvedValue({ kind: "unavailable" });
  vi.mocked(installUpdate).mockResolvedValue(undefined);
  // What the real ones answer in a browser: no folder chooser, no login item,
  // and `false` from the writer because nothing was written.
  vi.mocked(pickFolder).mockResolvedValue(null);
  vi.mocked(getAutostart).mockResolvedValue(null);
  vi.mocked(setAutostart).mockResolvedValue(false);
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("App integration", () => {
  // Every other test in this file renders <Settings> directly (see
  // SettingsHost above) so the new tests can address a specific section.
  // That change removed the only coverage of App's own wiring — the header
  // gear actually opening the dialog, App's setSettingsOpen(false) actually
  // closing it — so this one test keeps that path real.
  it("opens from the header's gear icon and closes for real", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const dialog = await screen.findByRole("dialog", { name: "Settings" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Close settings" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument()
    );
  });

  // Review finding on Task 6: the palette's Ctrl+K listener is bound to
  // `document` unconditionally, so it can open right over an already-visible
  // Settings panel. Before the fix, Settings read its section from a
  // mount-time useState initializer, so retargeting it while it stayed
  // mounted (React reconciles the same instance — settingsOpen was already
  // true) was silently a no-op: the click closed the palette and changed
  // nothing. Settings' section now lives in App as a single controlled
  // value instead, so this proves the *visible* section actually changes —
  // commandPalette.test.tsx's "deep-links into a settings section" only
  // proves onOpenSettings was called, with Settings never mounted at all.
  it("a palette deep link changes the section of an already-open Settings panel", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const settingsDialog = await screen.findByRole("dialog", { name: "Settings" });
    expect(within(settingsDialog).getByRole("heading", { name: "General" })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "k", ctrlKey: true });
    const paletteInput = await screen.findByPlaceholderText("Jump to…");
    const paletteDialog = paletteInput.closest('[role="dialog"]') as HTMLElement;
    fireEvent.click(within(paletteDialog).getByText("Storage & Privacy"));

    await waitFor(() =>
      expect(within(settingsDialog).getByRole("heading", { name: "Storage & Privacy" })).toBeInTheDocument()
    );
    // Same DOM node as before the palette interaction — proving the panel's
    // own state moved, not that a remount (e.g. a `key` on <Settings>)
    // happened to pick up the new section.
    expect(screen.getByRole("dialog", { name: "Settings" })).toBe(settingsDialog);
  });
});

describe("Settings screen", () => {
  it("checks for a desktop update only when asked", async () => {
    vi.mocked(checkForUpdate).mockResolvedValue({ kind: "current" });
    const dialog = await openSettings({ initialSection: "updates" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Check for updates" }));

    await waitFor(() => expect(checkForUpdate).toHaveBeenCalledOnce());
    // `findByText`, not `getByText`: the waitFor above resolves the moment the
    // *call* happens, which is before the message it sets has rendered. That
    // gap is real — it went red once in a loaded full-suite run — and the
    // assertion is otherwise identical.
    expect(await within(dialog).findByText("You’re up to date.")).toBeInTheDocument();
  });

  it("downloads a found update and restarts when requested", async () => {
    const update = {
      version: "0.1.2",
      body: "Faster recording meter.",
      downloadAndInstall: vi.fn(),
    };
    vi.mocked(checkForUpdate).mockResolvedValue({ kind: "available", update });
    const dialog = await openSettings({ initialSection: "updates" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Check for updates" }));
    expect(await within(dialog).findByText("Version 0.1.2 is ready.")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Download and restart" }));

    await waitFor(() => expect(installUpdate).toHaveBeenCalledWith(update, expect.any(Function)));
  });

  it("loads and displays the current settings when opened", async () => {
    const dialog = await openSettings({ initialSection: "storage" });
    // Called at least once by Settings itself.
    expect(api.getSettings).toHaveBeenCalled();
    expect(within(dialog).getByLabelText("Where recordings are saved")).toHaveValue(
      "/Users/george/Notetaker"
    );

    fireEvent.click(within(dialog).getByRole("button", { name: "Models & AI" }));
    fireEvent.click(within(dialog).getByText("Advanced AI service"));
    expect(await within(dialog).findByLabelText("AI service address")).toHaveValue("http://localhost:11434");
    expect(within(dialog).getByLabelText("Summary AI model")).toHaveValue("qwen2.5:7b");
  });

  it("reveals the log file's folder when asked", async () => {
    const dialog = await openSettings({ initialSection: "storage" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Open the log folder" }));
    await waitFor(() =>
      expect(revealItemInDir).toHaveBeenCalledWith(
        "/Users/george/Library/Notetaker/logs/notetaker.log",
      ),
    );
  });

  it("persists the storage location when the field is edited and blurred", async () => {
    const dialog = await openSettings({ initialSection: "storage" });
    const input = within(dialog).getByLabelText("Where recordings are saved");
    fireEvent.change(input, { target: { value: "/Users/george/Elsewhere" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, storageRoot: "/Users/george/Elsewhere" })
    );
  });

  it("persists the AI service address when edited and blurred", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    const input = within(dialog).getByLabelText("AI service address");
    fireEvent.change(input, { target: { value: "http://localhost:9999" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, llmBaseUrl: "http://localhost:9999" })
    );
  });

  it("persists the summary AI model name when edited and blurred", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    const input = within(dialog).getByLabelText("Summary AI model");
    fireEvent.change(input, { target: { value: "llama3:8b" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, llmModel: "llama3:8b" })
    );
  });

  it("shows the detected hardware tier", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    expect(await within(dialog).findByText(/Detected hardware: medium/)).toBeInTheDocument();
  });

  it("overriding the model size persists tierOverride, and returning to automatic clears it", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    const select = await within(dialog).findByLabelText("Model size override");

    fireEvent.change(select, { target: { value: "CpuBig" } });
    await waitFor(() =>
      expect(api.setSettings).toHaveBeenLastCalledWith({
        ...BASE_SETTINGS,
        performanceMode: "auto",
        tierOverride: "CpuBig",
      })
    );

    fireEvent.change(select, { target: { value: "" } });
    await waitFor(() =>
      expect(api.setSettings).toHaveBeenLastCalledWith({
        ...BASE_SETTINGS,
        tierOverride: null,
      })
    );
  });

  it("persists the pro performance mode and model keep-loaded duration", async () => {
    const dialog = await openSettings({ initialSection: "models" });

    fireEvent.change(within(dialog).getByLabelText("Performance mode"), {
      target: { value: "cpuOptimized" },
    });
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ performanceMode: "cpuOptimized" }),
    );

    fireEvent.change(within(dialog).getByLabelText("Keep speech models loaded"), {
      target: { value: "15m" },
    });
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ modelIdleUnload: "15m" }),
    );
  });

  it("offers a per-folder summary model and saves the override", async () => {
    setupApi({
      tasks: ["Client notes"],
      ollama: { ...OLLAMA_READY, models: ["qwen3:8b"] },
    });
    const dialog = await openSettings({ initialSection: "models" });
    const select = await within(dialog).findByLabelText("Summary model for Client notes");

    fireEvent.change(select, { target: { value: "qwen3:8b" } });
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ taskModels: { "Client notes": "qwen3:8b" } }),
    );
  });

  it("toggling 'wait until not in use' persists processWhenIdle", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    const checkbox = within(dialog).getByLabelText("Wait until I’m not using the computer");
    expect(checkbox).toBeChecked();

    fireEvent.click(checkbox);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, processWhenIdle: false })
    );
    expect(checkbox).not.toBeChecked();
  });

  it("the minutes input writes minIdleSecs in seconds", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    const input = within(dialog).getByLabelText("Minutes of inactivity before starting");
    expect(input).toHaveValue(5);

    fireEvent.change(input, { target: { value: "10" } });

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, minIdleSecs: 600 })
    );
  });

  it("toggling 'only process while plugged in' sends requireAc: false", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    const checkbox = within(dialog).getByLabelText("Only process while plugged in");
    expect(checkbox).toBeChecked();

    fireEvent.click(checkbox);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, requireAc: false })
    );
  });

  it("toggling 'keep the original recording file too' persists keepWav", async () => {
    const dialog = await openSettings({ initialSection: "storage" });
    const checkbox = within(dialog).getByLabelText("Keep the original recording file too");
    expect(checkbox).not.toBeChecked();

    fireEvent.click(checkbox);

    await waitFor(() => expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, keepWav: true }));
  });

  it("the theme select calls setPreference and never writes a persisted setting", async () => {
    const dialog = await openSettings();
    const select = within(dialog).getByLabelText("Theme");
    expect(select).toHaveValue("");

    fireEvent.change(select, { target: { value: "dark" } });

    await waitFor(() => expect(select).toHaveValue("dark"));
    // Theme lives in useTheme's own localStorage mechanism, never in the
    // Settings struct (design spec §6) — selecting one must not also fire
    // off a settings write.
    expect(api.setSettings).not.toHaveBeenCalled();
  });

  it("ticking Chinese persists it, which is what decides the extra download", async () => {
    const dialog = await openSettings();

    fireEvent.click(within(dialog).getByLabelText("Chinese (Mandarin)"));

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({
        ...BASE_SETTINGS,
        languages: ["en", "zh"],
      })
    );
  });

  it("unticking the last language falls back to English rather than none", async () => {
    // With nothing selected there is no basis for choosing a model at all, so
    // the app must not be able to reach that state.
    const dialog = await openSettings();

    fireEvent.click(within(dialog).getByLabelText("English"));

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, languages: ["en"] })
    );
  });

  it("forcing a single speech model persists the override", async () => {
    const dialog = await openSettings({ initialSection: "models" });

    fireEvent.change(within(dialog).getByLabelText("Speech model"), {
      target: { value: "whisper" },
    });

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, speechEngine: "whisper" })
    );
  });

  it("the microphone select shows a saved device even before the device list loads, and clearing it maps back to null", async () => {
    // No mock for listInputDevices here, so it resolves [] (as it always
    // does under jsdom, with no __TAURI_INTERNALS__) — the saved device id
    // has no matching <option> from the list itself. Without a fallback
    // <option> for it, the <select> would silently show "System default"
    // instead of the value that's actually saved.
    setupApi({ settings: { ...BASE_SETTINGS, inputDevice: "built-in-mic" } });
    const dialog = await openSettings({ initialSection: "audio" });
    const select = within(dialog).getByLabelText("Microphone");
    expect(select).toHaveValue("built-in-mic");

    fireEvent.change(select, { target: { value: "" } });

    await waitFor(() =>
      // "" on the wire must become null, not the empty string, per the
      // Settings contract's inputDevice: string | null.
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, inputDevice: null })
    );
  });

  it("persists the choice to keep dictation audio history", async () => {
    const dialog = await openSettings({ initialSection: "storage" });
    fireEvent.click(within(dialog).getByLabelText("Keep dictation audio history"));
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ dictationKeepAudio: true }),
    );
  });

  it("the auto-record three-way writes autoRecord: { zoom: 'always' }", async () => {
    const dialog = await openSettings({ initialSection: "meetings" });
    const zoomGroup = within(dialog).getByRole("group", { name: "Zoom" });
    fireEvent.click(within(zoomGroup).getByRole("radio", { name: "Always" }));

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, autoRecord: { zoom: "always" } })
    );
  });

  it("notes Google Meet cannot be auto-detected instead of offering a dead control", async () => {
    const dialog = await openSettings({ initialSection: "meetings" });
    expect(within(dialog).getByText(/Google Meet isn't in this list/)).toBeInTheDocument();
    expect(within(dialog).queryByRole("group", { name: "Google Meet" })).not.toBeInTheDocument();
  });

  it("adds a meeting-summary template and persists its headings", async () => {
    const settings = {
      ...BASE_SETTINGS,
      templates: [{
        id: "default",
        name: "General notes",
        blurb: "A good default for any conversation.",
        sections: "## TL;DR\n## Action items",
      }],
    };
    setupApi({ settings });
    const dialog = await openSettings({ initialSection: "meetings" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Add template" }));
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "Sales call" } });
    fireEvent.change(within(dialog).getByLabelText("Short description"), { target: { value: "Capture commitments" } });
    fireEvent.change(within(dialog).getByLabelText("Summary headings and instructions"), {
      target: { value: "## TL;DR\n## Commitments\n## Action items" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save template" }));

    await waitFor(() => expect(api.setSettings).toHaveBeenCalledWith({
      ...settings,
      templates: [
        ...settings.templates,
        {
          id: "sales_call",
          name: "Sales call",
          blurb: "Capture commitments",
          sections: "## TL;DR\n## Commitments\n## Action items",
        },
      ],
    }));
  });

  it("shows Ollama's install hint as guidance when it isn't installed", async () => {
    setupApi({ ollama: OLLAMA_NOT_INSTALLED });
    const dialog = await openSettings({ initialSection: "models" });

    expect(await within(dialog).findByText(OLLAMA_NOT_INSTALLED.installHint as string)).toBeInTheDocument();
    expect(within(dialog).getByText("Not installed")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: /pull/i })).not.toBeInTheDocument();
  });

  it("tells someone whose Ollama is stopped to open it, not to download it", async () => {
    setupApi({ ollama: OLLAMA_STOPPED });
    const dialog = await openSettings({ initialSection: "models" });

    expect(await within(dialog).findByText(OLLAMA_STOPPED.installHint as string)).toBeInTheDocument();
    expect(within(dialog).getByText("Installed, not running")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: /pull/i })).not.toBeInTheDocument();
  });

  it("shows Ollama as ready when running with the configured model present", async () => {
    setupApi({ ollama: OLLAMA_READY });
    const dialog = await openSettings({ initialSection: "models" });

    expect(await within(dialog).findByText("Ready")).toBeInTheDocument();
  });

  it("clicking Pull calls pullModel with the configured model and renders live progress", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    let resolveProgress: (v: PullProgress[]) => void = () => {};
    vi.mocked(api.pullProgress).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveProgress = resolve;
        })
    );

    fireEvent.click(within(dialog).getByRole("button", { name: "Pull model" }));

    await waitFor(() => expect(api.pullModel).toHaveBeenCalledWith("qwen2.5:7b"));

    resolveProgress([{ kind: "ollama", name: "qwen2.5:7b", percent: 42, error: null, done: false }]);

    expect(
      await within(dialog).findByRole("progressbar", { name: "qwen2.5:7b download progress" })
    ).toHaveAttribute("aria-valuenow", "42");
  });

  it("surfaces a failed pull's error text instead of leaving a stuck progress bar", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    vi.mocked(api.pullProgress).mockResolvedValue([
      { kind: "ollama", name: "qwen2.5:7b", percent: 10, error: "Connection refused", done: true },
    ]);

    fireEvent.click(within(dialog).getByRole("button", { name: "Pull model" }));

    expect(await within(dialog).findByText("Connection refused")).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("progressbar", { name: "qwen2.5:7b download progress" })
    ).not.toBeInTheDocument();
  });

  it("the Download button downloads missing models and refreshes setup status", async () => {
    const dialog = await openSettings({ initialSection: "models" });
    expect(within(dialog).getByText("Speech model (fast) (190 MB)")).toBeInTheDocument();

    vi.mocked(api.setupStatus).mockResolvedValue({
      ...SETUP_WITHOUT_FOUND_MODELS,
      missing: [],
      downloadBytes: 0,
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Download speech models" }));

    await waitFor(() => expect(api.downloadModels).toHaveBeenCalled());
    await waitFor(() =>
      expect(within(dialog).queryByText("Speech model (fast) (190 MB)")).not.toBeInTheDocument()
    );
  });

  it("renders all hotkey rows with their current accelerator, formatted into Kbd parts", async () => {
    const dialog = await openSettings({ initialSection: "shortcuts" });

    // Each row is an ARIA group labeled with its own row label (HotkeyField
    // renders role="group" aria-label={label} on the row) — a stable,
    // semantic hook. Scoping via .closest("div") would silently match a
    // different ancestor the moment the markup grows another wrapper.
    const toggleRow = within(dialog).getByRole("group", { name: "Start / stop recording" });
    expect(within(toggleRow).getByText("Works anywhere, even with the window closed")).toBeInTheDocument();
    expect(
      within(toggleRow).getByRole("button", { name: "Change shortcut: Start / stop recording" })
    ).toBeInTheDocument();
    expect(within(toggleRow).getByText("N")).toBeInTheDocument();

    const showHideRow = within(dialog).getByRole("group", { name: "Show / hide Notetaker" });
    expect(within(showHideRow).getByText("Brings the window up from the tray")).toBeInTheDocument();
    expect(
      within(showHideRow).getByRole("button", { name: "Change shortcut: Show / hide Notetaker" })
    ).toBeInTheDocument();
    expect(within(showHideRow).getByText("Space")).toBeInTheDocument();

    const highlightRow = within(dialog).getByRole("group", { name: "Star this moment" });
    expect(
      within(highlightRow).getByText("Bookmarks the current moment of a live recording — the AI treats starred moments as important")
    ).toBeInTheDocument();
    expect(
      within(highlightRow).getByRole("button", { name: "Change shortcut: Star this moment" })
    ).toBeInTheDocument();
    expect(within(highlightRow).getByText("H")).toBeInTheDocument();

    // All four rows share the CommandOrControl+Alt prefix, formatted for display
    // via formatAcceleratorParts — under jsdom's non-Mac navigator that's
    // "Ctrl", not the raw stored "CommandOrControl". Four bindable actions are
    // shown, including the saved dictation shortcut.
    expect(within(dialog).getAllByText("Ctrl")).toHaveLength(4);
    expect(within(dialog).getAllByText("Alt")).toHaveLength(4);
    expect(within(dialog).getByText("Dictate and paste")).toBeInTheDocument();
    expect(within(dialog).queryByText("CommandOrControl")).not.toBeInTheDocument();
  });

  it("the recorder button's accessible name announces listening state, for screen readers", async () => {
    const dialog = await openSettings({ initialSection: "shortcuts" });
    const btn = within(dialog).getByRole("button", { name: "Change shortcut: Start / stop recording" });
    expect(btn).toHaveAccessibleName("Change shortcut: Start / stop recording");

    fireEvent.click(btn);

    // A sighted user sees "Press the keys…" render inside the button; a
    // screen-reader user gets nothing unless the accessible name changes
    // too, since aria-label otherwise wins over visible content for the
    // accessible name computation. Same approved string either way (design
    // spec §5's "Hotkey recorder behavior").
    expect(btn).toHaveAccessibleName("Press the keys…");
  });

  it("records a new start/stop hotkey from a chord", async () => {
    const dialog = await openSettings({ initialSection: "shortcuts" });
    const btn = within(dialog).getByRole("button", { name: "Change shortcut: Start / stop recording" });
    fireEvent.click(btn);
    fireEvent.keyDown(btn, { key: "r", code: "KeyR", ctrlKey: true, altKey: true });
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ hotkeyToggleRecord: "CommandOrControl+Alt+R" }),
    );
  });

  it("can be closed with the close button", async () => {
    const dialog = await openSettings();
    fireEvent.click(within(dialog).getByRole("button", { name: "Close settings" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument());
  });
});

/**
 * The two controls that only exist inside the desktop shell: the login item and
 * the folder chooser. Neither has anything to act on in the served UI — a phone
 * on the LAN has no Startup Apps list and no filesystem to browse — so both are
 * gated on `isDesktop()`, which reads `__TAURI_INTERNALS__` off `window`. It is
 * stubbed here the way `transport.test.ts` and `capture.test.tsx` do, and
 * removed afterwards so desktop mode never leaks into a neighbouring test.
 */
describe("the desktop-only controls", () => {
  beforeEach(() => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    vi.mocked(getAutostart).mockResolvedValue(false);
    vi.mocked(setAutostart).mockResolvedValue(true);
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("turns start-with-Windows on from General", async () => {
    const dialog = await openSettings();

    const toggle = await within(dialog).findByRole("switch", {
      name: "Start Notetaker when you sign in",
    });
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);

    await waitFor(() => expect(setAutostart).toHaveBeenCalledWith(true));
    expect(toggle).toBeChecked();
  });

  /**
   * The chosen folder has to land in *both* places. The text box is a draft
   * seeded when settings loaded, so leaving it behind would not merely look
   * stale — the next blur commits the draft, writing the old path back over
   * the folder just chosen.
   */
  it("a chosen folder becomes the storage root, in the field as well as the setting", async () => {
    vi.mocked(pickFolder).mockResolvedValue("/Users/george/Recordings");
    const dialog = await openSettings({ initialSection: "storage" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Choose folder…" }));

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({
        ...BASE_SETTINGS,
        storageRoot: "/Users/george/Recordings",
      })
    );
    const input = within(dialog).getByLabelText("Where recordings are saved");
    expect(input).toHaveValue("/Users/george/Recordings");

    fireEvent.blur(input);
    expect(vi.mocked(api.setSettings).mock.calls).toHaveLength(1);
  });

  it("changes nothing when the folder chooser is cancelled", async () => {
    vi.mocked(pickFolder).mockResolvedValue(null);
    const dialog = await openSettings({ initialSection: "storage" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Choose folder…" }));

    await waitFor(() => expect(pickFolder).toHaveBeenCalled());
    expect(api.setSettings).not.toHaveBeenCalled();
    expect(within(dialog).getByLabelText("Where recordings are saved")).toHaveValue(
      "/Users/george/Notetaker"
    );
  });

  it("offers neither of them in the served UI", async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    vi.mocked(getAutostart).mockResolvedValue(null);
    const dialog = await openSettings();

    expect(
      within(dialog).queryByRole("switch", { name: "Start Notetaker when you sign in" })
    ).not.toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Storage & Privacy" }));
    await within(dialog).findByRole("heading", { name: "Storage & Privacy" });
    expect(
      within(dialog).queryByRole("button", { name: "Choose folder…" })
    ).not.toBeInTheDocument();
  });
});

describe("sectioned navigation", () => {
  it("shows the nine target sections and opens General by default", async () => {
    const dialog = await openSettings();
    const nav = within(dialog).getByRole("navigation", { name: "Settings sections" });
    for (const label of ["General", "Shortcuts", "Audio", "Models & AI", "Dictation", "Overlay", "Meetings", "Storage & Privacy", "Updates"]) {
      expect(within(nav).getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(within(dialog).getByRole("heading", { name: "General" })).toBeInTheDocument();
  });

  it("clicking a section shows that section's controls", async () => {
    const dialog = await openSettings();
    fireEvent.click(within(dialog).getByRole("button", { name: "Storage & Privacy" }));
    expect(await within(dialog).findByLabelText("Where recordings are saved")).toBeInTheDocument();
  });

  it("initialSection opens the asked-for section", async () => {
    const dialog = await openSettings({ initialSection: "updates" });
    expect(within(dialog).getByRole("button", { name: "Check for updates" })).toBeInTheDocument();
  });

  it("close-to-tray switch reflects and writes the setting", async () => {
    const dialog = await openSettings();
    const sw = within(dialog).getByRole("switch", { name: "Close button hides to tray" });
    expect(sw).toHaveAttribute("data-state", "checked");
    fireEvent.click(sw);
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ closeToTray: false }),
    );
  });

  it("filters the section list with search", async () => {
    const dialog = await openSettings();
    const search = within(dialog).getByPlaceholderText("Search settings");
    fireEvent.change(search, { target: { value: "microphone" } });
    const nav = within(dialog).getByRole("navigation", { name: "Settings sections" });

    expect(within(nav).getByRole("button", { name: "Audio" })).toBeInTheDocument();
    expect(within(nav).queryByRole("button", { name: "General" })).not.toBeInTheDocument();
  });
});

describe("First-run checklist", () => {
  it("states plainly that permissions are handled by the operating system", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    expect(within(card).getByText("System permissions")).toBeInTheDocument();
    expect(within(card).getByText("Handled by your operating system")).toBeInTheDocument();
  });

  it("reads the Ollama item as done when Ollama is running with the model present", async () => {
    setupApi({ ollama: OLLAMA_READY });
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });

    const item = within(card)
      .getByText("Install Ollama and download the summary model")
      .closest("li") as HTMLElement;
    await waitFor(() => expect(within(item).getByText("Done")).toBeInTheDocument());
  });

  it("tells someone whose Ollama is stopped to open it", async () => {
    setupApi({ ollama: OLLAMA_STOPPED });
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });

    expect(await within(card).findByText(OLLAMA_STOPPED.installHint as string)).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: /pull/i })).not.toBeInTheDocument();
  });

  it("asks which languages you speak before offering to download anything", async () => {
    // The order is the point: the answer decides what gets fetched, so it has
    // to be asked first or the download cannot be sized to the user.
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });

    const items = within(card).getAllByRole("listitem");
    const languageIndex = items.findIndex((li) =>
      li.textContent?.includes("Which languages do you speak?")
    );
    const downloadIndex = items.findIndex((li) =>
      li.textContent?.includes("Download the speech models")
    );

    expect(languageIndex).toBeGreaterThanOrEqual(0);
    expect(languageIndex).toBeLessThan(downloadIndex);
  });

  it("says nothing is transcribed until the models are downloaded", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });

    expect(
      within(card).getByText(/Nothing is transcribed until these are downloaded/i),
    ).toBeInTheDocument();
    expect(within(card).queryByText(/automatically in the background/i)).not.toBeInTheDocument();
  });

  it("recognizes speech models already on disk after restarting", async () => {
    setupApi({
      setup: { ...SETUP_WITHOUT_FOUND_MODELS, transcribing: true, missing: [], downloadBytes: 0 },
    });
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    const item = within(card).getByText("Download the speech models").closest("li") as HTMLElement;

    await waitFor(() => expect(within(item).getByText("Done")).toBeInTheDocument());
    expect(within(item).queryByRole("button", { name: "Download speech models" })).not.toBeInTheDocument();
    expect(api.findExistingModels).not.toHaveBeenCalled();
  });

  it("offers a found model for explicit adoption instead of another download", async () => {
    setupApi({ found: [{ name: "whisper-small-q5_1", label: "Speech model (fast)" }] });
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });

    expect(await within(card).findByText(/Found a copy of this on your computer/i)).toBeInTheDocument();
    fireEvent.click(within(card).getByRole("button", { name: "Use it instead" }));
    await waitFor(() => expect(api.adoptModels).toHaveBeenCalledOnce());
  });

  it("choosing a language on first run saves it without leaving the checklist", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    await waitFor(() => expect(within(card).getByLabelText("Japanese")).toBeInTheDocument());

    fireEvent.click(within(card).getByLabelText("Japanese"));

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({
        ...BASE_SETTINGS,
        languages: ["en", "ja"],
      })
    );
  });

  it("says when a choice costs an extra download and when it does not", async () => {
    // A 239 MB download is worth one sentence of warning before it starts.
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    await waitFor(() => expect(within(card).getByLabelText("Korean")).toBeInTheDocument());

    expect(within(card).getByText(/Nothing extra to download/i)).toBeInTheDocument();

    fireEvent.click(within(card).getByLabelText("Korean"));

    await waitFor(() =>
      expect(within(card).getByText(/second speech model \(about 239 MB\)/i)).toBeInTheDocument()
    );
  });

  it("surfaces a failed speech-model download's error text on its checklist item", async () => {
    vi.mocked(api.pullProgress).mockResolvedValue([
      { kind: "speech", name: "sensevoice-small", percent: 30, error: "Network error — check your connection.", done: true },
    ]);
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });

    expect(await within(card).findByText(/Network error — check your connection\./)).toBeInTheDocument();
  });

  it("clicking the speech-models button calls downloadModels", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    const button = await within(card).findByRole("button", { name: "Download speech models" });

    fireEvent.click(button);

    await waitFor(() => expect(api.downloadModels).toHaveBeenCalled());
  });

  it("renders progress with kind 'speech' on the speech item, not the Ollama item", async () => {
    vi.mocked(api.pullProgress).mockResolvedValue([
      { kind: "speech", name: "sensevoice-small", percent: 55, error: null, done: false },
    ]);
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    const speechItem = within(card).getByText("Download the speech models").closest("li") as HTMLElement;
    const ollamaItem = within(card)
      .getByText("Install Ollama and download the summary model")
      .closest("li") as HTMLElement;

    expect(await within(speechItem).findByText(/sensevoice-small — 55%/)).toBeInTheDocument();
    expect(within(ollamaItem).queryByText(/sensevoice-small/)).not.toBeInTheDocument();
  });

  it("renders progress with kind 'ollama' on the Ollama item, not the speech item", async () => {
    vi.mocked(api.pullProgress).mockResolvedValue([
      { kind: "ollama", name: "qwen2.5:7b", percent: 20, error: null, done: false },
    ]);
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    const speechItem = within(card).getByText("Download the speech models").closest("li") as HTMLElement;
    const ollamaItem = within(card)
      .getByText("Install Ollama and download the summary model")
      .closest("li") as HTMLElement;

    expect(await within(ollamaItem).findByText(/qwen2\.5:7b — 20%/)).toBeInTheDocument();
    expect(within(speechItem).queryByText(/qwen2\.5:7b/)).not.toBeInTheDocument();
  });

  it("classifies a speech model matching none of the old heuristic's name substrings (paraformer-zh) as speech", async () => {
    // Proves the `name`-substring heuristic is gone: "paraformer-zh" contains
    // none of the old hints (whisper/sense/sherpa/diariz/pyannote), so this
    // would have shown "Not started" forever under the old code.
    vi.mocked(api.pullProgress).mockResolvedValue([
      { kind: "speech", name: "paraformer-zh", percent: 10, error: null, done: false },
    ]);
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    const speechItem = within(card).getByText("Download the speech models").closest("li") as HTMLElement;

    expect(await within(speechItem).findByText(/paraformer-zh — 10%/)).toBeInTheDocument();
    await waitFor(() => expect(within(speechItem).getByText("In progress")).toBeInTheDocument());
  });

  it("an errored speech download surfaces its message and the button becomes available again", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    const item = within(card).getByText("Download the speech models").closest("li") as HTMLElement;

    vi.mocked(api.pullProgress).mockResolvedValue([
      {
        kind: "speech",
        name: "sensevoice-small",
        percent: 40,
        error: "Network error — check your connection.",
        done: true,
      },
    ]);

    fireEvent.click(within(item).getByRole("button", { name: "Download speech models" }));

    expect(await within(item).findByText(/Network error — check your connection\./)).toBeInTheDocument();
    expect(within(item).queryByRole("progressbar")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(within(item).getByRole("button", { name: "Download speech models" })).toBeEnabled()
    );
  });

  it("can be dismissed, and the app underneath remains fully usable", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    fireEvent.click(within(card).getByRole("button", { name: "Dismiss" }));

    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Getting started" })).not.toBeInTheDocument()
    );

    vi.mocked(api.startCapture).mockResolvedValue({
      state: "recording",
      mode: "meeting",
      recordingId: "rec-1",
      elapsedS: 0,
      micLevel: 0,
      systemLevel: 0,
      diskFreeMb: 20_000,
    });
    const startButton = await screen.findByRole("button", { name: "Record" });
    expect(startButton).toBeEnabled();
    fireEvent.click(startButton);

    await waitFor(() => expect(api.startCapture).toHaveBeenCalledWith("meeting", expect.any(String)));
  });
});
