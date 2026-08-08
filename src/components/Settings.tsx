import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, LANGUAGE_CHOICES } from "../lib/ipc";
import { checkForUpdate, installUpdate } from "../lib/updater";
import type { PendingUpdate, UpdateProgress } from "../lib/updater";
import { getAutostart, listInputDevices, pickFolder, setAutostart } from "../lib/desktop";
import type { InputDevice } from "../lib/desktop";
import { isDesktop } from "../lib/transport";
import { formatBytes } from "../lib/format";
import type { useTheme, ThemePreference } from "../hooks/useTheme";
import type { HotkeyIssues } from "../hooks/useGlobalHotkeys";
import { Button, Notice, Switch } from "./ui";
import { HotkeyField } from "./HotkeyField";
import { cn } from "../lib/cn";
import type {
  AutoRecordPolicy,
  DictationMode,
  ModelIdleUnload,
  OllamaStatus,
  OverlayMode,
  OverlayPosition,
  OverlayStyle,
  PasteBehavior,
  PerformanceMode,
  PullProgress,
  Settings as SettingsData,
  SetupStatus,
  SpeechEngine,
  SummaryTemplate,
} from "../lib/ipc";

export type SettingsSection =
  | "general"
  | "shortcuts"
  | "audio"
  | "models"
  | "dictation"
  | "overlay"
  | "meetings"
  | "storage"
  | "updates";

export const SECTIONS: Array<{ id: SettingsSection; label: string }> = [
  { id: "general", label: "General" },
  { id: "shortcuts", label: "Shortcuts" },
  { id: "audio", label: "Audio" },
  { id: "models", label: "Models & AI" },
  { id: "dictation", label: "Dictation" },
  { id: "overlay", label: "Overlay" },
  { id: "meetings", label: "Meetings" },
  { id: "storage", label: "Storage & Privacy" },
  { id: "updates", label: "Updates" },
];

const SECTION_SEARCH_TERMS: Record<SettingsSection, string> = {
  general: "theme languages launch tray startup",
  shortcuts: "hotkeys keyboard recording show hide star dictation",
  audio: "microphone mic device priority level meter test",
  models: "speech transcription ollama download model ai cleanup summary performance memory idle",
  dictation: "dictionary replacements push to talk toggle paste clipboard",
  overlay: "floating pill position style glass screen share capture",
  meetings: "zoom teams slack webex discord facetime automatic record policy",
  storage: "folder recordings audio wav logs privacy local",
  updates: "version update download restart",
};

const NAV_ITEM_BASE =
  "w-full rounded-[var(--radius-control)] px-3 py-1.5 text-left text-[13px] font-medium transition-colors";
const NAV_ITEM_ACTIVE = "bg-selected text-fg shadow-[inset_2px_0_0_var(--c-accent)]";
const NAV_ITEM_IDLE = "text-fg-muted hover:bg-hover hover:text-fg";

export interface SettingsProps {
  onClose: () => void;
  theme: ReturnType<typeof useTheme>;
  section: SettingsSection;
  onSelectSection: (s: SettingsSection) => void;
  // The hook's own type, imported rather than restated: a structural copy would
  // go on compiling after a third issue is added, and the panel would simply
  // never show it.
  hotkeyIssues?: HotkeyIssues;
  /** Called once a write has actually landed. */
  onSaved?: () => void;
}

const PULL_POLL_MS = 700;

/** The meeting watcher ids that can be selected in auto-record policies. */
const KNOWN_APPS: Array<{ id: string; label: string }> = [
  { id: "zoom", label: "Zoom" },
  { id: "teams", label: "Microsoft Teams" },
  { id: "slack", label: "Slack" },
  { id: "webex", label: "Webex" },
  { id: "discord", label: "Discord" },
  { id: "facetime", label: "FaceTime" },
  { id: "call", label: "Any other call (microphone in use)" },
];

const POLICY_OPTIONS: Array<{ value: AutoRecordPolicy; label: string }> = [
  { value: "ask", label: "Ask" },
  { value: "always", label: "Always" },
  { value: "never", label: "Never" },
];

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * The Rust side supplies these values for old settings files. Keeping the
 * same safety net in the webview makes an older desktop shell or a test
 * fixture degrade to the documented defaults instead of crashing the panel.
 */
function normalizeSettings(value: SettingsData): SettingsData {
  const candidate = value as Partial<SettingsData>;
  return {
    ...value,
    taskModels: candidate.taskModels ?? {},
    templates: candidate.templates ?? [],
    audioDevicePriority: candidate.audioDevicePriority ?? [],
    performanceMode: candidate.performanceMode ?? "auto",
    modelIdleUnload: candidate.modelIdleUnload ?? "5m",
    cleanupModel: candidate.cleanupModel ?? "llama3.2:3b",
    dictationCleanupEnabled: candidate.dictationCleanupEnabled ?? true,
    dictationDictionary: candidate.dictationDictionary ?? [],
    dictationReplacements: candidate.dictationReplacements ?? {},
    dictationMode: candidate.dictationMode ?? "pushToTalk",
    dictationPasteBehavior: candidate.dictationPasteBehavior ?? "paste",
    dictationHotkey: candidate.dictationHotkey ?? "CommandOrControl+Alt+D",
    dictationKeepAudio: candidate.dictationKeepAudio ?? false,
    overlayPosition: candidate.overlayPosition ?? "topRight",
    overlayStyle: candidate.overlayStyle ?? "glass",
    overlayHideFromShare: candidate.overlayHideFromShare ?? true,
  };
}

function ollamaStatusLabel(status: OllamaStatus | null): string {
  if (!status) return "Checking…";
  if (!status.installed) return "Not installed";
  if (!status.running) return "Installed, not running";
  if (!status.modelReady) return "Installed — model not downloaded yet";
  return "Ready";
}

function ollamaStatusKind(status: OllamaStatus | null): "recorded" | "processing" | "ready" {
  if (!status || !status.installed) return "recorded";
  if (!status.modelReady) return "processing";
  return "ready";
}

function humanTier(tier: string | null): string {
  if (!tier) return "checking…";
  return tier
    .replace(/^AppleSilicon/, "Apple Silicon ")
    .replace(/^Cpu/, "CPU ")
    .replace(/Big$/, "large")
    .replace(/Small$/, "small");
}

function PullBar({ entry, fallbackName }: { entry: PullProgress | undefined; fallbackName: string }) {
  if (entry?.error) {
    return (
      <p className="settings-pull__error" role="alert">
        {entry.error}
      </p>
    );
  }
  const name = entry?.name ?? fallbackName;
  const pct = Math.max(0, Math.min(100, Math.round(entry?.percent ?? 0)));
  return (
    <div className="progress-bar">
      <span className="progress-bar__label">
        {name} — {pct}%
      </span>
      <span
        className="progress-bar__track"
        role="progressbar"
        aria-label={`${name} download progress`}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <span className="progress-bar__fill" style={{ width: `${pct}%` }} />
      </span>
    </div>
  );
}

export function Settings({
  onClose,
  theme,
  section,
  onSelectSection,
  hotkeyIssues,
  onSaved,
}: SettingsProps) {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [detectedTier, setDetectedTier] = useState<string | null>(null);
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);
  const [setup, setSetup] = useState<SetupStatus | null>(null);
  const [taskNames, setTaskNames] = useState<string[]>([]);
  const [inputDevices, setInputDevices] = useState<InputDevice[]>([]);
  const [autostart, setAutostartState] = useState<boolean | null>(null);
  const [progress, setProgress] = useState<PullProgress[]>([]);
  const [pulling, setPulling] = useState(false);
  const [downloadingModels, setDownloadingModels] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [update, setUpdate] = useState<PendingUpdate | null>(null);
  const [updateMessage, setUpdateMessage] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress | null>(null);
  const [settingsSearch, setSettingsSearch] = useState("");

  const [storageDraft, setStorageDraft] = useState("");
  const [baseUrlDraft, setBaseUrlDraft] = useState("");
  const [modelDraft, setModelDraft] = useState("");
  const [dictionaryDraft, setDictionaryDraft] = useState("");
  const [replacementsDraft, setReplacementsDraft] = useState("");
  const [editingTemplateId, setEditingTemplateId] = useState<string | null>(null);
  const [templateNameDraft, setTemplateNameDraft] = useState("");
  const [templateBlurbDraft, setTemplateBlurbDraft] = useState("");
  const [templateSectionsDraft, setTemplateSectionsDraft] = useState("");

  const [micTesting, setMicTesting] = useState(false);
  const [micLevel, setMicLevel] = useState(0);
  const [micTestError, setMicTestError] = useState<string | null>(null);
  const micStream = useRef<MediaStream | null>(null);
  const micContext = useRef<AudioContext | null>(null);
  const micFrame = useRef<number | null>(null);

  const panelRef = useRef<HTMLDivElement | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);
  const pullTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    closeButtonRef.current?.focus();
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const loaded = await api.getSettings();
        if (cancelled) return;
        const normalized = loaded ? normalizeSettings(loaded) : null;
        setSettings(normalized);
        if (normalized) {
          setStorageDraft(normalized.storageRoot);
          setBaseUrlDraft(normalized.llmBaseUrl);
          setModelDraft(normalized.llmModel);
          setDictionaryDraft(normalized.dictationDictionary.join("\n"));
          setReplacementsDraft(
            Object.entries(normalized.dictationReplacements)
              .map(([from, to]) => `${from} => ${to}`)
              .join("\n"),
          );
        }
      } catch (err) {
        if (!cancelled) setLoadError(describeError(err));
      }
    })();
    (async () => {
      try {
        const tier = await api.detectedTier();
        if (!cancelled) setDetectedTier(tier ?? null);
      } catch (err) {
        if (!cancelled) setLoadError(describeError(err));
      }
    })();
    (async () => {
      try {
        const status = await api.ollamaStatus();
        if (!cancelled) setOllama(status ?? null);
      } catch (err) {
        if (!cancelled) setLoadError(describeError(err));
      }
    })();
    (async () => {
      try {
        const next = await api.setupStatus();
        if (!cancelled) setSetup(next ?? null);
      } catch (err) {
        if (!cancelled) setLoadError(describeError(err));
      }
    })();
    (async () => {
      try {
        const tasks = await api.listTasks();
        if (!cancelled) setTaskNames(tasks ?? []);
      } catch {
        // The task list is optional context for the model picker. A failed
        // read must not make the rest of Settings unusable.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    listInputDevices().then((devices) => {
      if (!cancelled) setInputDevices(devices ?? []);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    getAutostart().then((on) => {
      if (!cancelled) setAutostartState(on);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const stopMicTest = useCallback(async () => {
    if (micFrame.current !== null) {
      cancelAnimationFrame(micFrame.current);
      micFrame.current = null;
    }
    micStream.current?.getTracks().forEach((track) => track.stop());
    micStream.current = null;
    if (micContext.current) {
      await micContext.current.close().catch(() => undefined);
      micContext.current = null;
    }
    setMicTesting(false);
    setMicLevel(0);
  }, []);

  useEffect(() => () => void stopMicTest(), [stopMicTest]);

  const updateSettings = useCallback(
    async (next: SettingsData) => {
      setSettings(next);
      try {
        await api.setSettings(next);
        onSaved?.();
        return true;
      } catch (err) {
        setLoadError(describeError(err));
        return false;
      }
    },
    [onSaved],
  );

  const refreshPullProgress = useCallback(async () => {
    try {
      const next = await api.pullProgress();
      setProgress(next ?? []);
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

  useEffect(() => {
    if (!pulling) return;
    void refreshPullProgress();
    pullTimer.current = setInterval(() => void refreshPullProgress(), PULL_POLL_MS);
    return () => {
      if (pullTimer.current) {
        clearInterval(pullTimer.current);
        pullTimer.current = null;
      }
    };
  }, [pulling, refreshPullProgress]);

  const pullEntry = settings ? progress.find((p) => p.name === settings.llmModel) : undefined;

  useEffect(() => {
    if (!pullEntry?.done) return;
    setPulling(false);
    api
      .ollamaStatus()
      .then((s) => setOllama(s ?? null))
      .catch((err) => setLoadError(describeError(err)));
  }, [pullEntry?.done]);

  async function handlePull() {
    if (!settings) return;
    setLoadError(null);
    setPulling(true);
    try {
      await api.pullModel(settings.llmModel);
    } catch (err) {
      setLoadError(describeError(err));
      setPulling(false);
    }
  }

  async function handleDownloadModels() {
    setLoadError(null);
    setDownloadingModels(true);
    try {
      await api.downloadModels();
      setSetup((await api.setupStatus()) ?? null);
    } catch (err) {
      setLoadError(describeError(err));
    } finally {
      setDownloadingModels(false);
    }
  }

  async function handleCheckForUpdate() {
    setCheckingUpdate(true);
    setUpdate(null);
    setUpdateProgress(null);
    setUpdateMessage(null);
    try {
      const result = await checkForUpdate();
      if (result.kind === "available") setUpdate(result.update);
      else if (result.kind === "current") setUpdateMessage("You’re up to date.");
      else setUpdateMessage("Updates are available in the installed desktop app.");
    } catch (err) {
      setUpdateMessage(`Couldn’t check for updates: ${describeError(err)}`);
    } finally {
      setCheckingUpdate(false);
    }
  }

  async function handleInstallUpdate() {
    if (!update) return;
    setInstallingUpdate(true);
    setUpdateMessage("Downloading update…");
    try {
      await installUpdate(update, setUpdateProgress);
      setUpdateMessage("Update installed. Restarting Notetaker…");
    } catch (err) {
      setUpdateMessage(`Couldn’t install the update: ${describeError(err)}`);
      setInstallingUpdate(false);
    }
  }

  async function chooseStorageFolder() {
    const dir = await pickFolder();
    if (!dir || !settings) return;
    setStorageDraft(dir);
    await updateSettings({ ...settings, storageRoot: dir });
  }

  function commitStorage() {
    if (!settings) return;
    const trimmed = storageDraft.trim();
    if (trimmed && trimmed !== settings.storageRoot) void updateSettings({ ...settings, storageRoot: trimmed });
    else setStorageDraft(settings.storageRoot);
  }

  async function openLogFolder() {
    try {
      await revealItemInDir(await api.logPath());
    } catch (err) {
      setLoadError(describeError(err));
    }
  }

  function commitBaseUrl() {
    if (!settings) return;
    const trimmed = baseUrlDraft.trim();
    if (trimmed && trimmed !== settings.llmBaseUrl) void updateSettings({ ...settings, llmBaseUrl: trimmed });
    else setBaseUrlDraft(settings.llmBaseUrl);
  }

  function commitModel() {
    if (!settings) return;
    const trimmed = modelDraft.trim();
    if (trimmed && trimmed !== settings.llmModel) void updateSettings({ ...settings, llmModel: trimmed });
    else setModelDraft(settings.llmModel);
  }

  function commitDictionary() {
    if (!settings) return;
    const words = Array.from(
      new Set(dictionaryDraft.split(/\r?\n/).map((word) => word.trim()).filter(Boolean)),
    );
    setDictionaryDraft(words.join("\n"));
    if (words.join("\n") !== settings.dictationDictionary.join("\n")) {
      void updateSettings({ ...settings, dictationDictionary: words });
    }
  }

  function commitReplacements() {
    if (!settings) return;
    const replacements: Record<string, string> = {};
    for (const line of replacementsDraft.split(/\r?\n/)) {
      const match = line.match(/^\s*(.*?)\s*=>\s*(.*?)\s*$/);
      if (match?.[1] && match[2]) replacements[match[1]] = match[2];
    }
    setReplacementsDraft(Object.entries(replacements).map(([from, to]) => `${from} => ${to}`).join("\n"));
    if (JSON.stringify(replacements) !== JSON.stringify(settings.dictationReplacements)) {
      void updateSettings({ ...settings, dictationReplacements: replacements });
    }
  }

  function handleAutoRecordChange(appId: string, policy: AutoRecordPolicy) {
    if (!settings) return;
    void updateSettings({
      ...settings,
      autoRecord: { ...(settings.autoRecord ?? {}), [appId]: policy },
    });
  }

  function resetTemplateEditor() {
    setEditingTemplateId(null);
    setTemplateNameDraft("");
    setTemplateBlurbDraft("");
    setTemplateSectionsDraft("## TL;DR (2-3 sentences)\n## Key points\n## Decisions\n## Action items (checkbox list, each starting with the owner's name and a colon)\n## Open questions");
  }

  function startTemplateEdit(template: SummaryTemplate) {
    setEditingTemplateId(template.id);
    setTemplateNameDraft(template.name);
    setTemplateBlurbDraft(template.blurb);
    setTemplateSectionsDraft(template.sections);
  }

  function templateIdFor(name: string) {
    const base = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") || "meeting_summary";
    if (!settings?.templates.some((template) => template.id === base)) return base;
    let suffix = 2;
    while (settings.templates.some((template) => template.id === `${base}_${suffix}`)) suffix += 1;
    return `${base}_${suffix}`;
  }

  async function saveTemplate() {
    if (!settings) return;
    const name = templateNameDraft.trim();
    const blurb = templateBlurbDraft.trim();
    const sections = templateSectionsDraft.trim();
    if (!name || !blurb || !sections) {
      setLoadError("Give the template a name, a short description, and its summary headings.");
      return;
    }
    const id = editingTemplateId ?? templateIdFor(name);
    const nextTemplate = { id, name, blurb, sections };
    const templates = editingTemplateId
      ? settings.templates.map((template) => (template.id === id ? nextTemplate : template))
      : [...settings.templates, nextTemplate];
    if (await updateSettings({ ...settings, templates })) resetTemplateEditor();
  }

  async function deleteTemplate(template: SummaryTemplate) {
    if (!settings || template.id === "default") return;
    if (!window.confirm(`Delete the "${template.name}" template? Existing recordings will use General notes if you process them again.`)) return;
    if (await updateSettings({ ...settings, templates: settings.templates.filter((item) => item.id !== template.id) })) {
      if (editingTemplateId === template.id) resetTemplateEditor();
    }
  }

  function moveDevice(deviceId: string, direction: -1 | 1) {
    if (!settings) return;
    const ids = (settings.audioDevicePriority.length > 0
      ? settings.audioDevicePriority
      : inputDevices.map((device) => device.id)
    ).filter((id, index, list) => list.indexOf(id) === index);
    const index = ids.indexOf(deviceId);
    const nextIndex = index + direction;
    if (index < 0 || nextIndex < 0 || nextIndex >= ids.length) return;
    [ids[index], ids[nextIndex]] = [ids[nextIndex], ids[index]];
    void updateSettings({ ...settings, audioDevicePriority: ids, inputDevice: ids[0] ?? null });
  }

  async function startMicTest() {
    await stopMicTest();
    setMicTestError(null);
    if (!navigator.mediaDevices?.getUserMedia) {
      setMicTestError("Microphone testing is not available in this browser.");
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const AudioContextConstructor =
        window.AudioContext ??
        (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!AudioContextConstructor) {
        stream.getTracks().forEach((track) => track.stop());
        setMicTestError("Microphone testing is not available in this browser.");
        return;
      }
      const context = new AudioContextConstructor();
      const source = context.createMediaStreamSource(stream);
      const analyser = context.createAnalyser();
      analyser.fftSize = 256;
      source.connect(analyser);
      const samples = new Uint8Array(analyser.fftSize);
      micStream.current = stream;
      micContext.current = context;
      setMicTesting(true);

      const readLevel = () => {
        analyser.getByteTimeDomainData(samples);
        let sum = 0;
        for (const sample of samples) {
          const normalized = (sample - 128) / 128;
          sum += normalized * normalized;
        }
        setMicLevel(Math.min(1, Math.sqrt(sum / samples.length) * 3.5));
        micFrame.current = requestAnimationFrame(readLevel);
      };
      readLevel();
    } catch (err) {
      setMicTestError(`Couldn’t start the microphone test: ${describeError(err)}`);
    }
  }

  function trapFocus(e: ReactKeyboardEvent<HTMLDivElement>) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key !== "Tab") return;
    const container = panelRef.current;
    if (!container) return;
    const focusables = Array.from(
      container.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href]',
      ),
    );
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  const visibleSections = useMemo(() => {
    const query = settingsSearch.trim().toLowerCase();
    if (!query) return SECTIONS;
    return SECTIONS.filter((candidate) =>
      `${candidate.label} ${SECTION_SEARCH_TERMS[candidate.id]}`.toLowerCase().includes(query),
    );
  }, [settingsSearch]);

  const orderedDevices = useMemo(() => {
    if (inputDevices.length === 0) return [];
    const ids = settings?.audioDevicePriority?.length
      ? settings.audioDevicePriority
      : inputDevices.map((device) => device.id);
    return ids
      .map((id) => inputDevices.find((device) => device.id === id))
      .filter((device): device is InputDevice => device !== undefined);
  }, [inputDevices, settings?.audioDevicePriority]);

  const installedModels = ollama?.models ?? [];
  const modelOptions = Array.from(new Set([settings?.llmModel ?? "", ...installedModels].filter(Boolean)));
  const cleanupModelOptions = Array.from(
    new Set([settings?.cleanupModel ?? "llama3.2:3b", ...installedModels].filter(Boolean)),
  );

  return (
    <div className="settings-overlay">
      <div
        className="settings-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        ref={panelRef}
        onKeyDown={trapFocus}
      >
        <header className="settings-panel__header">
          <div>
            <p className="settings-panel__eyebrow">Preferences</p>
            <h2 id="settings-title">Settings</h2>
            <p className="settings-panel__subtitle">Tune recording, models, and the way Notetaker fits your day.</p>
          </div>
          <button
            type="button"
            className="settings-panel__close"
            onClick={onClose}
            aria-label="Close settings"
            ref={closeButtonRef}
          >
            ×
          </button>
        </header>

        <div className="settings-layout">
          <nav aria-label="Settings sections" className="settings-nav">
            <label className="settings-search">
              <span className="sr-only">Search settings</span>
              <span aria-hidden="true" className="settings-search__icon">⌕</span>
              <input
                type="search"
                placeholder="Search settings"
                value={settingsSearch}
                onChange={(e) => setSettingsSearch(e.target.value)}
              />
            </label>
            <div className="settings-nav__list">
              {visibleSections.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => onSelectSection(s.id)}
                  aria-current={section === s.id ? "true" : undefined}
                  className={cn(NAV_ITEM_BASE, section === s.id ? NAV_ITEM_ACTIVE : NAV_ITEM_IDLE)}
                >
                  {s.label}
                </button>
              ))}
            </div>
            {settingsSearch && visibleSections.length === 0 && (
              <p className="settings-nav__empty">No section matches that search.</p>
            )}
            <p className="settings-nav__footer">Changes save as you make them.</p>
          </nav>

          <div className="settings-panel__body">
            {loadError && <Notice className="mb-3">{loadError}</Notice>}

            {section === "updates" ? (
              <section aria-labelledby="settings-heading-updates" className="settings-section">
                <h3 id="settings-heading-updates">Updates</h3>
                <p className="settings-section__lede">
                  Check for a signed Notetaker update. Nothing is downloaded until you choose to install it.
                </p>
                <div className="settings-update">
                  <button
                    type="button"
                    onClick={() => void handleCheckForUpdate()}
                    disabled={checkingUpdate || installingUpdate}
                  >
                    {checkingUpdate ? "Checking for updates…" : "Check for updates"}
                  </button>
                  {update && (
                    <div className="settings-update__available">
                      <p role="status">Version {update.version} is ready.</p>
                      {update.body && <p className="settings-hint">{update.body}</p>}
                      <button type="button" onClick={() => void handleInstallUpdate()} disabled={installingUpdate}>
                        {installingUpdate ? "Downloading update…" : "Download and restart"}
                      </button>
                    </div>
                  )}
                  {updateProgress && (
                    <div className="progress-bar" aria-live="polite">
                      <span className="progress-bar__label">
                        {updateProgress.total
                          ? `${Math.min(100, Math.round((updateProgress.received / updateProgress.total) * 100))}% downloaded`
                          : "Downloading update…"}
                      </span>
                      {updateProgress.total && (
                        <span
                          className="progress-bar__track"
                          role="progressbar"
                          aria-label="Update download progress"
                          aria-valuenow={Math.min(
                            100,
                            Math.round((updateProgress.received / updateProgress.total) * 100),
                          )}
                          aria-valuemin={0}
                          aria-valuemax={100}
                        >
                          <span
                            className="progress-bar__fill"
                            style={{ width: `${Math.min(100, (updateProgress.received / updateProgress.total) * 100)}%` }}
                          />
                        </span>
                      )}
                    </div>
                  )}
                  {updateMessage && (
                    <p className="settings-hint settings-update__message" role="status">
                      {updateMessage}
                    </p>
                  )}
                </div>
              </section>
            ) : !settings ? (
              <p aria-live="polite">Loading settings…</p>
            ) : (
              <>
                {section === "general" && (
                  <section aria-labelledby="settings-heading-general" className="settings-section">
                    <h3 id="settings-heading-general">General</h3>
                    <p className="settings-section__lede">A few choices that shape the whole app.</p>

                    <div className="settings-field">
                      <label htmlFor="settings-theme">Theme</label>
                      <select
                        id="settings-theme"
                        value={theme.preference === "system" ? "" : theme.preference}
                        onChange={(e) =>
                          theme.setPreference(e.target.value === "" ? "system" : (e.target.value as ThemePreference))
                        }
                      >
                        <option value="">System</option>
                        <option value="light">Light</option>
                        <option value="dark">Dark</option>
                      </select>
                    </div>

                    <div className="settings-field settings-field--checkbox">
                      <Switch
                        checked={settings.closeToTray}
                        onCheckedChange={(v) => void updateSettings({ ...settings, closeToTray: v })}
                        label="Close button hides to tray"
                      />
                      <span className="settings-field__label-text">Close button hides to tray</span>
                    </div>

                    {isDesktop() && autostart !== null && (
                      <div className="settings-field settings-field--checkbox">
                        <Switch
                          checked={autostart}
                          onCheckedChange={(v) => {
                            setAutostartState(v);
                            void setAutostart(v);
                          }}
                          label="Start Notetaker when you sign in"
                        />
                        <span className="settings-field__label-text">Start Notetaker when you sign in</span>
                      </div>
                    )}

                    <div className="settings-field">
                      <span className="settings-field__label-text">Languages in recordings</span>
                      <p className="settings-hint">
                        Pick every language you expect to hear. The choice decides whether the extra speech model is downloaded.
                      </p>
                      <div className="settings-languages">
                        {LANGUAGE_CHOICES.map((choice) => (
                          <label key={choice.code} className="settings-language">
                            <input
                              type="checkbox"
                              checked={(settings.languages ?? ["en"]).includes(choice.code)}
                              onChange={(e) => {
                                const current = settings.languages ?? ["en"];
                                const next = e.target.checked
                                  ? [...current, choice.code]
                                  : current.filter((code) => code !== choice.code);
                                void updateSettings({
                                  ...settings,
                                  languages: next.length === 0 ? ["en"] : next,
                                });
                              }}
                            />
                            <span>{choice.label}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </section>
                )}

                {section === "shortcuts" && (
                  <section aria-labelledby="settings-heading-shortcuts" className="settings-section">
                    <h3 id="settings-heading-shortcuts">Shortcuts</h3>
                    <p className="settings-section__lede">
                      Every shortcut is global, so it works while the main window is hidden. Conflicts are called out on the row that needs attention.
                    </p>
                    <div className="settings-hotkeys">
                      <HotkeyField
                        label="Start / stop recording"
                        hint="Works anywhere, even with the window closed"
                        value={settings.hotkeyToggleRecord}
                        issue={hotkeyIssues?.toggleRecord ?? null}
                        onChange={(accelerator) => void updateSettings({ ...settings, hotkeyToggleRecord: accelerator })}
                      />
                      <HotkeyField
                        label="Show / hide Notetaker"
                        hint="Brings the window up from the tray"
                        value={settings.hotkeyShowHide}
                        issue={hotkeyIssues?.showHide ?? null}
                        onChange={(accelerator) => void updateSettings({ ...settings, hotkeyShowHide: accelerator })}
                      />
                      <HotkeyField
                        label="Star this moment"
                        hint="Bookmarks the current moment of a live recording — the AI treats starred moments as important"
                        value={settings.hotkeyHighlight}
                        issue={hotkeyIssues?.highlight ?? null}
                        onChange={(accelerator) => void updateSettings({ ...settings, hotkeyHighlight: accelerator })}
                      />
                      <HotkeyField
                        label="Dictate and paste"
                        hint="Hold this anywhere to speak into the active cursor"
                        value={settings.dictationHotkey}
                        issue={hotkeyIssues?.dictation ?? null}
                        onChange={(accelerator) => void updateSettings({ ...settings, dictationHotkey: accelerator })}
                      />
                    </div>
                    <p className="settings-hint settings-hint--action">
                      All four shortcuts are checked with the operating system. Dictation uses the microphone, local speech models, and the clipboard; it never sends audio to a server.
                    </p>
                  </section>
                )}

                {section === "audio" && (
                  <section aria-labelledby="settings-heading-audio" className="settings-section">
                    <h3 id="settings-heading-audio">Audio</h3>
                    <p className="settings-section__lede">Choose the microphone to use and verify that it is hearing you.</p>

                    <div className="settings-field">
                      <label htmlFor="settings-input-device">Microphone</label>
                      <select
                        id="settings-input-device"
                        value={settings.inputDevice ?? ""}
                        onChange={(e) =>
                          void updateSettings({
                            ...settings,
                            inputDevice: e.target.value === "" ? null : e.target.value,
                          })
                        }
                      >
                        <option value="">System default</option>
                        {settings.inputDevice && !inputDevices.some((device) => device.id === settings.inputDevice) && (
                          <option value={settings.inputDevice}>{settings.inputDevice}</option>
                        )}
                        {inputDevices.map((device) => (
                          <option key={device.id} value={device.id}>
                            {device.label}{device.isDefault ? " (default)" : ""}
                          </option>
                        ))}
                      </select>
                      <p className="settings-hint">The selected device is used first. If it is unavailable, the priority list below is used.</p>
                    </div>

                    {orderedDevices.length > 0 && (
                      <fieldset className="settings-device-priority">
                        <legend>Device priority</legend>
                        <p className="settings-hint">Move a device up to make it the next fallback.</p>
                        <ol>
                          {orderedDevices.map((device, index) => (
                            <li key={device.id}>
                              <span>{device.label}</span>
                              <span className="settings-device-priority__actions">
                                <button
                                  type="button"
                                  aria-label={`Move ${device.label} up`}
                                  onClick={() => moveDevice(device.id, -1)}
                                  disabled={index === 0}
                                >
                                  ↑
                                </button>
                                <button
                                  type="button"
                                  aria-label={`Move ${device.label} down`}
                                  onClick={() => moveDevice(device.id, 1)}
                                  disabled={index === orderedDevices.length - 1}
                                >
                                  ↓
                                </button>
                              </span>
                            </li>
                          ))}
                        </ol>
                      </fieldset>
                    )}

                    <div className="settings-field settings-mic-test">
                      <span className="settings-field__label-text">Microphone test</span>
                      <p className="settings-hint">Speak for a moment. The meter stays on this screen and is not saved.</p>
                      <div className="progress-bar settings-level-meter">
                        <span className="progress-bar__label">Level</span>
                        <span
                          className="progress-bar__track"
                          role="progressbar"
                          aria-label="Microphone level"
                          aria-valuenow={Math.round(micLevel * 100)}
                          aria-valuemin={0}
                          aria-valuemax={100}
                        >
                          <span className="progress-bar__fill" style={{ width: `${Math.round(micLevel * 100)}%` }} />
                        </span>
                      </div>
                      <div className="settings-inline-actions">
                        <Button size="sm" onClick={() => void startMicTest()} disabled={micTesting}>
                          Test microphone
                        </Button>
                        {micTesting && (
                          <Button variant="secondary" size="sm" onClick={() => void stopMicTest()}>
                            Stop test
                          </Button>
                        )}
                      </div>
                      {micTesting && <p className="settings-hint" role="status">Listening…</p>}
                      {micTestError && <p className="settings-pull__error" role="alert">{micTestError}</p>}
                    </div>
                  </section>
                )}

                {section === "models" && (
                  <section aria-labelledby="settings-heading-models" className="settings-section">
                    <h3 id="settings-heading-models">Models &amp; AI</h3>
                    <p className="settings-section__lede">Choose the balance between quality, speed, memory, and battery life.</p>

                    <div className="settings-field">
                      <label htmlFor="settings-performance-mode">Performance mode</label>
                      <select
                        id="settings-performance-mode"
                        value={settings.performanceMode}
                        onChange={(e) =>
                          void updateSettings({
                            ...settings,
                            performanceMode: e.target.value as PerformanceMode,
                            tierOverride: e.target.value === "auto" ? null : settings.tierOverride,
                          })
                        }
                      >
                        <option value="auto">Auto — use detected hardware</option>
                        <option value="bestQuality">Best Quality — use the largest available model</option>
                        <option value="cpuOptimized">CPU-Optimized — use less memory and processor time</option>
                      </select>
                      <p className="settings-hint">
                        Auto uses the detected model tier and your battery preference. Best Quality favors accuracy; CPU-Optimized favors a lighter model.
                      </p>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-tier-override">Model size override</label>
                      <p className="settings-hint">Detected hardware: {humanTier(detectedTier)}. Use this only when you want a specific tier.</p>
                      <select
                        id="settings-tier-override"
                        value={settings.tierOverride ?? ""}
                        onChange={(e) =>
                          void updateSettings({
                            ...settings,
                            performanceMode: "auto",
                            tierOverride: e.target.value === "" ? null : e.target.value,
                          })
                        }
                      >
                        <option value="">Use the performance mode</option>
                        <option value="AppleSiliconSmall">Small — fastest, least memory</option>
                        <option value="AppleSiliconBig">Large — highest quality on Apple Silicon</option>
                        <option value="CpuSmall">Small — fastest CPU option</option>
                        <option value="CpuBig">Large — highest quality CPU option</option>
                      </select>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-speech-engine">Speech model</label>
                      <select
                        id="settings-speech-engine"
                        value={settings.speechEngine ?? "auto"}
                        onChange={(e) => void updateSettings({ ...settings, speechEngine: e.target.value as SpeechEngine })}
                      >
                        <option value="auto">Automatic — choose per sentence</option>
                        <option value="whisper">Always use Whisper</option>
                        <option value="senseVoice">Always use SenseVoice</option>
                      </select>
                      <p className="settings-hint">Automatic uses the model that best fits each language.</p>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-model-idle-unload">Keep speech models loaded</label>
                      <select
                        id="settings-model-idle-unload"
                        value={settings.modelIdleUnload}
                        onChange={(e) => void updateSettings({ ...settings, modelIdleUnload: e.target.value as ModelIdleUnload })}
                      >
                        <option value="never">Never unload</option>
                        <option value="afterBatch">After the current batch</option>
                        <option value="2m">For 2 minutes</option>
                        <option value="5m">For 5 minutes</option>
                        <option value="15m">For 15 minutes</option>
                        <option value="1h">For 1 hour</option>
                      </select>
                      <p className="settings-hint">Longer keeps the next transcript quick; shorter returns memory to the rest of the computer sooner.</p>
                    </div>

                    <div className="settings-model-card">
                      <div className="settings-model-card__header">
                        <div>
                          <h4>Speech model downloads</h4>
                          <p className="settings-hint">Models stay on this computer and are checked against their expected files.</p>
                        </div>
                        <span className={`status-chip status-chip--${setup?.missing.length ? "processing" : "ready"}`}>
                          <span className="status-chip__dot" aria-hidden="true" />
                          {setup?.missing.length ? `${setup.missing.length} to download` : "Ready"}
                        </span>
                      </div>
                      {setup?.missing.length ? (
                        <>
                          <div className="settings-model-list">
                            {setup.missing.map((model) => (
                              <div className="settings-model-list__row" key={model.name}>
                                <span>{model.label} ({formatBytes(model.bytes)})</span>
                              </div>
                            ))}
                          </div>
                          <Button size="sm" onClick={() => void handleDownloadModels()} disabled={downloadingModels}>
                            {downloadingModels ? "Downloading…" : "Download speech models"}
                          </Button>
                        </>
                      ) : (
                        <p className="settings-hint">The speech models needed for {humanTier(setup?.tier ?? detectedTier)} are ready.</p>
                      )}
                    </div>

                    <div className="settings-model-card">
                      <div className="settings-model-card__header">
                        <div>
                          <h4>Local AI (Ollama)</h4>
                          <p className="settings-hint">Ollama writes summaries and cleanup text locally.</p>
                        </div>
                        <span className={`status-chip status-chip--${ollamaStatusKind(ollama)}`}>
                          <span className="status-chip__dot" aria-hidden="true" />
                          {ollamaStatusLabel(ollama)}
                        </span>
                      </div>
                      {ollama?.installHint && <p className="settings-hint settings-hint--action">{ollama.installHint}</p>}
                      <div className="settings-field">
                        <label htmlFor="settings-llm-model">Summary AI model</label>
                        <input
                          id="settings-llm-model"
                          type="text"
                          value={modelDraft}
                          onChange={(e) => setModelDraft(e.target.value)}
                          onBlur={commitModel}
                        />
                        {modelOptions.length > 0 && (
                          <select
                            aria-label="Installed summary models"
                            value={settings.llmModel}
                            onChange={(e) => {
                              setModelDraft(e.target.value);
                              void updateSettings({ ...settings, llmModel: e.target.value });
                            }}
                          >
                            {modelOptions.map((model) => <option key={model} value={model}>{model}</option>)}
                          </select>
                        )}
                      </div>
                      {ollama?.running && (
                        <div className="settings-pull">
                          <button type="button" onClick={() => void handlePull()} disabled={pulling}>
                            {ollama.modelReady ? "Pull again" : "Pull model"}
                          </button>
                          {(pulling || pullEntry?.error) && <PullBar entry={pullEntry} fallbackName={settings.llmModel} />}
                        </div>
                      )}
                    </div>

                    <div className="settings-model-card settings-task-models">
                      <div className="settings-model-card__header">
                        <div>
                          <h4>Models by note folder</h4>
                          <p className="settings-hint">Choose a summary model for each folder. Folders without a choice use the Summary AI model above.</p>
                        </div>
                      </div>
                      {taskNames.length === 0 ? (
                        <p className="settings-hint">Create a note folder to give it its own summary model.</p>
                      ) : (
                        <div className="settings-task-models__list">
                          {taskNames.map((taskName) => (
                            <div className="settings-task-models__row" key={taskName}>
                              <label htmlFor={`settings-task-model-${taskName}`}>{taskName}</label>
                              <select
                                id={`settings-task-model-${taskName}`}
                                aria-label={`Summary model for ${taskName}`}
                                value={settings.taskModels[taskName] ?? ""}
                                onChange={(e) => {
                                  const taskModels = { ...settings.taskModels };
                                  if (e.target.value) taskModels[taskName] = e.target.value;
                                  else delete taskModels[taskName];
                                  void updateSettings({ ...settings, taskModels });
                                }}
                              >
                                <option value="">Use Summary AI model</option>
                                {modelOptions.map((model) => <option key={model} value={model}>{model}</option>)}
                              </select>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-cleanup-model">Dictation cleanup model</label>
                      <select
                        id="settings-cleanup-model"
                        value={settings.cleanupModel}
                        onChange={(e) => void updateSettings({ ...settings, cleanupModel: e.target.value })}
                      >
                        {cleanupModelOptions.map((model) => <option key={model} value={model}>{model}</option>)}
                      </select>
                      <p className="settings-hint">A small model keeps cleanup quick. It is used only when dictation cleanup is enabled.</p>
                    </div>

                    <div className="settings-field settings-field--checkbox">
                      <input
                        type="checkbox"
                        id="settings-dictation-cleanup"
                        checked={settings.dictationCleanupEnabled}
                        onChange={(e) => void updateSettings({ ...settings, dictationCleanupEnabled: e.target.checked })}
                      />
                      <label htmlFor="settings-dictation-cleanup">Clean up dictation text with a local model</label>
                    </div>

                    <div className="settings-field">
                      <span className="settings-field__label-text">Background processing</span>
                      <div className="settings-field settings-field--checkbox">
                        <input
                          type="checkbox"
                          id="settings-process-when-idle"
                          checked={settings.processWhenIdle}
                          onChange={(e) => void updateSettings({ ...settings, processWhenIdle: e.target.checked })}
                        />
                        <label htmlFor="settings-process-when-idle">Wait until I’m not using the computer</label>
                      </div>
                      <label htmlFor="settings-min-idle-minutes">Minutes of inactivity before starting</label>
                      <input
                        id="settings-min-idle-minutes"
                        type="number"
                        min={1}
                        step={1}
                        value={Math.max(1, Math.round(settings.minIdleSecs / 60))}
                        onChange={(e) => {
                          const minutes = Number(e.target.value);
                          if (Number.isFinite(minutes) && minutes > 0) {
                            void updateSettings({ ...settings, minIdleSecs: Math.round(minutes * 60) });
                          }
                        }}
                      />
                      <div className="settings-field settings-field--checkbox">
                        <input
                          type="checkbox"
                          id="settings-require-ac"
                          checked={settings.requireAc}
                          onChange={(e) => void updateSettings({ ...settings, requireAc: e.target.checked })}
                        />
                        <label htmlFor="settings-require-ac">Only process while plugged in</label>
                      </div>
                    </div>

                    <details>
                      <summary className="settings-field__label-text">Advanced AI service</summary>
                      <div className="settings-field">
                        <label htmlFor="settings-llm-base-url">AI service address</label>
                        <p className="settings-hint">Leave this as-is unless a different local service was deliberately configured.</p>
                        <input
                          id="settings-llm-base-url"
                          type="url"
                          value={baseUrlDraft}
                          onChange={(e) => setBaseUrlDraft(e.target.value)}
                          onBlur={commitBaseUrl}
                        />
                      </div>
                    </details>
                  </section>
                )}

                {section === "dictation" && (
                  <section aria-labelledby="settings-heading-dictation" className="settings-section">
                    <h3 id="settings-heading-dictation">Dictation</h3>
                    <p className="settings-section__lede">Prepare the words and paste behavior used by system-wide dictation.</p>
                    <p className="settings-hint settings-hint--action">
                      Dictation is active anywhere on the computer. Short utterances stay deterministic; longer ones use the selected local cleanup model.
                    </p>

                    <div className="settings-field">
                      <label htmlFor="settings-dictation-mode">How dictation starts</label>
                      <select
                        id="settings-dictation-mode"
                        value={settings.dictationMode}
                        onChange={(e) => void updateSettings({ ...settings, dictationMode: e.target.value as DictationMode })}
                      >
                        <option value="pushToTalk">Push to talk — hold the shortcut while speaking</option>
                        <option value="toggle">Toggle — press once to start and again to finish</option>
                      </select>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-dictation-paste">After dictation finishes</label>
                      <select
                        id="settings-dictation-paste"
                        value={settings.dictationPasteBehavior}
                        onChange={(e) => void updateSettings({ ...settings, dictationPasteBehavior: e.target.value as PasteBehavior })}
                      >
                        <option value="paste">Paste at the active cursor</option>
                        <option value="copyOnly">Copy to the clipboard only</option>
                      </select>
                      <p className="settings-hint">When paste permissions are unavailable, text should remain available to copy rather than disappearing.</p>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-dictionary">Dictionary words and names</label>
                      <textarea
                        id="settings-dictionary"
                        value={dictionaryDraft}
                        onChange={(e) => setDictionaryDraft(e.target.value)}
                        onBlur={commitDictionary}
                        rows={5}
                        placeholder="One word or name per line"
                      />
                      <p className="settings-hint">One word or name per line. These entries help recognition; they are kept locally.</p>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-replacements">Replacements</label>
                      <textarea
                        id="settings-replacements"
                        value={replacementsDraft}
                        onChange={(e) => setReplacementsDraft(e.target.value)}
                        onBlur={commitReplacements}
                        rows={5}
                        placeholder="spoken phrase => corrected text"
                      />
                      <p className="settings-hint">One rule per line, using <code>spoken phrase =&gt; corrected text</code>.</p>
                    </div>
                  </section>
                )}

                {section === "overlay" && (
                  <section aria-labelledby="settings-heading-overlay" className="settings-section">
                    <h3 id="settings-heading-overlay">Overlay</h3>
                    <p className="settings-section__lede">Choose when the small recording control appears above other windows.</p>

                    <div className="settings-field">
                      <label htmlFor="settings-overlay">Overlay mode</label>
                      <select
                        id="settings-overlay"
                        value={settings.overlay ?? "recording"}
                        onChange={(e) => void updateSettings({ ...settings, overlay: e.target.value as OverlayMode })}
                      >
                        <option value="recording">While recording</option>
                        <option value="meeting">When a meeting is detected</option>
                        <option value="off">Never</option>
                      </select>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-overlay-position">Position</label>
                      <select
                        id="settings-overlay-position"
                        value={settings.overlayPosition}
                        onChange={(e) => void updateSettings({ ...settings, overlayPosition: e.target.value as OverlayPosition })}
                      >
                        <option value="topRight">Top right</option>
                        <option value="topCenter">Top center</option>
                        <option value="bottomCenter">Bottom center</option>
                      </select>
                      <p className="settings-hint">Your choice is saved now. Applying positions beyond the current top-right placement is ticketed for the desktop window-positioning pass. While dictating, the flow bar moves to bottom-center so it stays near the active cursor without covering the top edge.</p>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-overlay-style">Style</label>
                      <select
                        id="settings-overlay-style"
                        value={settings.overlayStyle}
                        onChange={(e) => void updateSettings({ ...settings, overlayStyle: e.target.value as OverlayStyle })}
                      >
                        <option value="glass">Glass</option>
                        <option value="solid">Solid</option>
                      </select>
                    </div>

                    <div className="settings-field settings-field--checkbox">
                      <Switch
                        checked={settings.overlayHideFromShare}
                        onCheckedChange={(v) => void updateSettings({ ...settings, overlayHideFromShare: v })}
                        label="Hide overlay from screen sharing"
                      />
                      <span className="settings-field__label-text">Hide overlay from screen sharing</span>
                    </div>
                    <p className="settings-hint">
                      Windows can honor this setting. On macOS 15.4 and later, meeting apps may still capture the overlay because the operating system does not provide a public guarantee.
                    </p>
                  </section>
                )}

                {section === "meetings" && (
                  <section aria-labelledby="settings-heading-meetings" className="settings-section">
                    <h3 id="settings-heading-meetings">Meetings</h3>
                    <p className="settings-section__lede">Choose what should happen when Notetaker recognizes a call.</p>
                    <p className="settings-hint">Ask every time is the default. Always starts without a prompt; Never ignores that app.</p>
                    {KNOWN_APPS.map((app) => (
                      <fieldset className="settings-app-policy" key={app.id}>
                        <legend>{app.label}</legend>
                        <div className="settings-app-policy__options">
                          {POLICY_OPTIONS.map((option) => {
                            const id = `autorecord-${app.id}-${option.value}`;
                            const checked = (settings.autoRecord?.[app.id] ?? "ask") === option.value;
                            return (
                              <span key={option.value} className="settings-app-policy__option">
                                <input
                                  type="radio"
                                  id={id}
                                  name={`autorecord-${app.id}`}
                                  checked={checked}
                                  onChange={() => handleAutoRecordChange(app.id, option.value)}
                                />
                                <label htmlFor={id}>{option.label}</label>
                              </span>
                            );
                          })}
                        </div>
                      </fieldset>
                    ))}
                    <p className="settings-hint">
                      Google Meet isn't in this list. A browser being open doesn't mean you're on a call, so we can't reliably detect Meet meetings yet — start that recording yourself when you join one.
                    </p>

                    <div className="mt-7 border-t border-border pt-5">
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <h4 className="text-[14px] font-semibold text-fg">Meeting summary templates</h4>
                          <p className="settings-hint">Set the headings and instructions the AI uses the next time a meeting is processed.</p>
                        </div>
                        {editingTemplateId === null && (
                          <Button type="button" variant="secondary" size="sm" onClick={resetTemplateEditor}>Add template</Button>
                        )}
                      </div>

                      <div className="mt-3 space-y-2">
                        {settings.templates.map((template) => (
                          <div key={template.id} className="rounded-[var(--radius-control)] border border-border bg-sunken px-3 py-2">
                            <div className="flex flex-wrap items-start justify-between gap-2">
                              <div>
                                <p className="text-[13px] font-medium text-fg">{template.name}</p>
                                <p className="text-[12px] text-fg-muted">{template.blurb}</p>
                              </div>
                              <div className="flex gap-2">
                                <Button type="button" variant="secondary" size="sm" onClick={() => startTemplateEdit(template)}>Edit</Button>
                                {template.id !== "default" && (
                                  <Button type="button" variant="danger" size="sm" onClick={() => void deleteTemplate(template)}>Delete</Button>
                                )}
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>

                      {templateSectionsDraft && (
                        <div className="mt-3 space-y-3 rounded-[var(--radius-control)] border border-border bg-raised p-3">
                          <p className="text-[13px] font-medium text-fg">{editingTemplateId ? "Edit template" : "New template"}</p>
                          <div className="settings-field">
                            <label htmlFor="settings-template-name">Name</label>
                            <input id="settings-template-name" value={templateNameDraft} onChange={(e) => setTemplateNameDraft(e.target.value)} />
                          </div>
                          <div className="settings-field">
                            <label htmlFor="settings-template-blurb">Short description</label>
                            <input id="settings-template-blurb" value={templateBlurbDraft} onChange={(e) => setTemplateBlurbDraft(e.target.value)} />
                          </div>
                          <div className="settings-field">
                            <label htmlFor="settings-template-sections">Summary headings and instructions</label>
                            <p className="settings-hint">Start each heading with <code>##</code> and include an Action items section.</p>
                            <textarea id="settings-template-sections" rows={9} value={templateSectionsDraft} onChange={(e) => setTemplateSectionsDraft(e.target.value)} />
                          </div>
                          <div className="flex gap-2">
                            <Button type="button" onClick={() => void saveTemplate()}>Save template</Button>
                            <Button type="button" variant="secondary" onClick={resetTemplateEditor}>Cancel</Button>
                          </div>
                        </div>
                      )}
                    </div>
                  </section>
                )}

                {section === "storage" && (
                  <section aria-labelledby="settings-heading-storage" className="settings-section">
                    <h3 id="settings-heading-storage">Storage &amp; Privacy</h3>
                    <p className="settings-section__lede">Keep recordings easy to find and know what stays on disk.</p>

                    <div className="settings-field">
                      <label htmlFor="settings-storage-root">Where recordings are saved</label>
                      <p className="settings-hint">Recordings, transcripts, and summaries are saved in this folder on the computer.</p>
                      <div className="settings-inline-field">
                        <input
                          id="settings-storage-root"
                          type="text"
                          value={storageDraft}
                          onChange={(e) => setStorageDraft(e.target.value)}
                          onBlur={commitStorage}
                        />
                        {isDesktop() && (
                          <Button variant="secondary" size="sm" className="shrink-0" onClick={() => void chooseStorageFolder()}>
                            Choose folder…
                          </Button>
                        )}
                      </div>
                    </div>

                    <div className="settings-field settings-field--checkbox">
                      <input
                        type="checkbox"
                        id="settings-keep-wav"
                        checked={settings.keepWav}
                        onChange={(e) => void updateSettings({ ...settings, keepWav: e.target.checked })}
                      />
                      <label htmlFor="settings-keep-wav">Keep the original recording file too</label>
                    </div>
                    <p className="settings-hint">The original is lossless but uses about twice the disk space. Turn this on when you need that extra copy.</p>

                    <div className="settings-field settings-field--checkbox">
                      <input
                        type="checkbox"
                        id="settings-dictation-keep-audio"
                        checked={settings.dictationKeepAudio}
                        onChange={(e) => void updateSettings({ ...settings, dictationKeepAudio: e.target.checked })}
                      />
                      <label htmlFor="settings-dictation-keep-audio">Keep dictation audio history</label>
                    </div>
                    <p className="settings-hint">Dictation text is kept locally either way. Turn this on only when you also want a lossless audio copy.</p>

                    <div className="settings-privacy-card">
                      <span className="settings-field__label-text">Private by default</span>
                      <p className="settings-hint">Recordings, transcripts, summaries, dictionary entries, and logs stay on this computer. Notetaker does not upload them.</p>
                    </div>

                    <div className="settings-field">
                      <button type="button" onClick={() => void openLogFolder()}>Open the log folder</button>
                      <p className="settings-hint">Logs help explain a problem and are stored locally. Open them only when you want to inspect or share them.</p>
                    </div>
                  </section>
                )}
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
