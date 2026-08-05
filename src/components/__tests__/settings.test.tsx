import { useState } from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, within, fireEvent, waitFor, cleanup } from "@testing-library/react";
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

vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("../../lib/updater", () => ({ checkForUpdate: vi.fn(), installUpdate: vi.fn() }));

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

const BASE_SETTINGS: Settings = {
  storageRoot: "/Users/george/Notetaker",
  llmBaseUrl: "http://localhost:11434",
  llmModel: "qwen2.5:7b",
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

function setupApi(overrides: { settings?: Settings; ollama?: OllamaStatus; found?: FoundModel[]; setup?: SetupStatus } = {}) {
  vi.mocked(api.listTasks).mockResolvedValue([]);
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
 * Now `initialSection` has to reach the component itself, and nothing in
 * `<App>` exposes that from the outside until the command palette's deep
 * links land in Task 6. `onClose` still does a real unmount (not a `vi.fn()`
 * stub) so "can be closed with the close button" keeps testing real close
 * behavior rather than a mock that was merely called.
 */
function SettingsHost(props: Partial<SettingsProps>) {
  const theme = useTheme();
  const [open, setOpen] = useState(true);
  if (!open) return null;
  return <SettingsComponent onClose={() => setOpen(false)} theme={theme} {...props} />;
}

const SECTION_HEADINGS: Record<SettingsSection, string> = {
  general: "General",
  recording: "Recording",
  hotkeys: "Hotkeys",
  ai: "Transcription & AI",
  storage: "Storage",
  updates: "Updates",
};

async function openSettings(props: Partial<SettingsProps> = {}) {
  render(<SettingsHost {...props} />);
  const dialog = await screen.findByRole("dialog", { name: "Settings" });
  // Wait for the async settings load to finish, and for the requested
  // section (General by default) to be the one actually showing, before
  // handing the dialog back to the test.
  await within(dialog).findByRole("heading", { name: SECTION_HEADINGS[props.initialSection ?? "general"] });
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
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("Settings screen", () => {
  it("checks for a desktop update only when asked", async () => {
    vi.mocked(checkForUpdate).mockResolvedValue({ kind: "current" });
    const dialog = await openSettings({ initialSection: "updates" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Check for updates" }));

    await waitFor(() => expect(checkForUpdate).toHaveBeenCalledOnce());
    expect(within(dialog).getByText("You’re up to date.")).toBeInTheDocument();
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

    fireEvent.click(within(dialog).getByRole("button", { name: "Transcription & AI" }));
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
    const dialog = await openSettings({ initialSection: "ai" });
    const input = within(dialog).getByLabelText("AI service address");
    fireEvent.change(input, { target: { value: "http://localhost:9999" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, llmBaseUrl: "http://localhost:9999" })
    );
  });

  it("persists the summary AI model name when edited and blurred", async () => {
    const dialog = await openSettings({ initialSection: "ai" });
    const input = within(dialog).getByLabelText("Summary AI model");
    fireEvent.change(input, { target: { value: "llama3:8b" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, llmModel: "llama3:8b" })
    );
  });

  it("shows the detected hardware tier", async () => {
    const dialog = await openSettings({ initialSection: "ai" });
    expect(await within(dialog).findByText(/Detected: medium/)).toBeInTheDocument();
  });

  it("overriding the model size persists tierOverride, and returning to automatic clears it", async () => {
    const dialog = await openSettings({ initialSection: "ai" });
    const select = await within(dialog).findByLabelText(/Detected: medium/);

    fireEvent.change(select, { target: { value: "large" } });
    await waitFor(() =>
      expect(api.setSettings).toHaveBeenLastCalledWith({ ...BASE_SETTINGS, tierOverride: "large" })
    );

    fireEvent.change(select, { target: { value: "" } });
    await waitFor(() =>
      expect(api.setSettings).toHaveBeenLastCalledWith({
        ...BASE_SETTINGS,
        tierOverride: null,
      })
    );
  });

  it("toggling 'wait until not in use' persists processWhenIdle", async () => {
    const dialog = await openSettings({ initialSection: "recording" });
    const checkbox = within(dialog).getByLabelText("Wait until I'm not using the computer");
    expect(checkbox).toBeChecked();

    fireEvent.click(checkbox);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, processWhenIdle: false })
    );
    expect(checkbox).not.toBeChecked();
  });

  it("the minutes input writes minIdleSecs in seconds", async () => {
    const dialog = await openSettings({ initialSection: "recording" });
    const input = within(dialog).getByLabelText("Minutes of inactivity before starting");
    expect(input).toHaveValue(5);

    fireEvent.change(input, { target: { value: "10" } });

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, minIdleSecs: 600 })
    );
  });

  it("toggling 'only process while plugged in' sends requireAc: false", async () => {
    const dialog = await openSettings({ initialSection: "recording" });
    const checkbox = within(dialog).getByLabelText("Only process while plugged in");
    expect(checkbox).toBeChecked();

    fireEvent.click(checkbox);

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, requireAc: false })
    );
  });

  it("toggling 'keep the original recording file too' persists keepWav", async () => {
    const dialog = await openSettings({ initialSection: "recording" });
    const checkbox = within(dialog).getByLabelText("Keep the original recording file too");
    expect(checkbox).not.toBeChecked();

    fireEvent.click(checkbox);

    await waitFor(() => expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, keepWav: true }));
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
    const dialog = await openSettings({ initialSection: "ai" });

    fireEvent.change(within(dialog).getByLabelText("Speech model"), {
      target: { value: "whisper" },
    });

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, speechEngine: "whisper" })
    );
  });

  it("the auto-record three-way writes autoRecord: { zoom: 'always' }", async () => {
    const dialog = await openSettings({ initialSection: "recording" });
    const zoomGroup = within(dialog).getByRole("group", { name: "Zoom" });
    fireEvent.click(within(zoomGroup).getByRole("radio", { name: "Always" }));

    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith({ ...BASE_SETTINGS, autoRecord: { zoom: "always" } })
    );
  });

  it("notes Google Meet cannot be auto-detected instead of offering a dead control", async () => {
    const dialog = await openSettings({ initialSection: "recording" });
    expect(within(dialog).getByText(/Google Meet isn't in this list/)).toBeInTheDocument();
    expect(within(dialog).queryByRole("group", { name: "Google Meet" })).not.toBeInTheDocument();
  });

  it("shows Ollama's install hint as guidance when it isn't installed", async () => {
    setupApi({ ollama: OLLAMA_NOT_INSTALLED });
    const dialog = await openSettings({ initialSection: "ai" });

    expect(await within(dialog).findByText(OLLAMA_NOT_INSTALLED.installHint as string)).toBeInTheDocument();
    expect(within(dialog).getByText("Not installed")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: /pull/i })).not.toBeInTheDocument();
  });

  it("tells someone whose Ollama is stopped to open it, not to download it", async () => {
    setupApi({ ollama: OLLAMA_STOPPED });
    const dialog = await openSettings({ initialSection: "ai" });

    expect(await within(dialog).findByText(OLLAMA_STOPPED.installHint as string)).toBeInTheDocument();
    expect(within(dialog).getByText("Installed, not running")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: /pull/i })).not.toBeInTheDocument();
  });

  it("shows Ollama as ready when running with the configured model present", async () => {
    setupApi({ ollama: OLLAMA_READY });
    const dialog = await openSettings({ initialSection: "ai" });

    expect(await within(dialog).findByText("Ready")).toBeInTheDocument();
  });

  it("clicking Pull calls pullModel with the configured model and renders live progress", async () => {
    const dialog = await openSettings({ initialSection: "ai" });
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
    const dialog = await openSettings({ initialSection: "ai" });
    vi.mocked(api.pullProgress).mockResolvedValue([
      { kind: "ollama", name: "qwen2.5:7b", percent: 10, error: "Connection refused", done: true },
    ]);

    fireEvent.click(within(dialog).getByRole("button", { name: "Pull model" }));

    expect(await within(dialog).findByText("Connection refused")).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("progressbar", { name: "qwen2.5:7b download progress" })
    ).not.toBeInTheDocument();
  });

  it("can be closed with the close button", async () => {
    const dialog = await openSettings();
    fireEvent.click(within(dialog).getByRole("button", { name: "Close settings" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument());
  });
});

describe("sectioned navigation", () => {
  it("shows six sections and opens General by default", async () => {
    const dialog = await openSettings();
    const nav = within(dialog).getByRole("navigation", { name: "Settings sections" });
    for (const label of ["General", "Recording", "Hotkeys", "Transcription & AI", "Storage", "Updates"]) {
      expect(within(nav).getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(within(dialog).getByRole("heading", { name: "General" })).toBeInTheDocument();
  });

  it("clicking a section shows that section's controls", async () => {
    const dialog = await openSettings();
    fireEvent.click(within(dialog).getByRole("button", { name: "Storage" }));
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
});

describe("First-run checklist", () => {
  it("states plainly that permissions are handled by the operating system", async () => {
    render(<App />);
    const card = await screen.findByRole("region", { name: "Getting started" });
    expect(within(card).getByText("Microphone and screen-recording permissions")).toBeInTheDocument();
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
