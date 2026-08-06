import { useCallback, useEffect, useRef, useState } from "react";
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
  OllamaStatus,
  OverlayMode,
  PullProgress,
  Settings as SettingsData,
  SetupStatus,
  SpeechEngine,
} from "../lib/ipc";

export type SettingsSection = "general" | "recording" | "hotkeys" | "ai" | "storage" | "updates";

export const SECTIONS: Array<{ id: SettingsSection; label: string }> = [
  { id: "general", label: "General" },
  { id: "recording", label: "Recording" },
  { id: "hotkeys", label: "Hotkeys" },
  { id: "ai", label: "Transcription & AI" },
  { id: "storage", label: "Storage" },
  { id: "updates", label: "Updates" },
];

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
  /**
   * Called once a write has actually landed. This is how a rebind reaches the
   * OS while the panel is still open — App refetches on it, which re-registers
   * the accelerator and puts any conflict back on the field you just used.
   */
  onSaved?: () => void;
}

const PULL_POLL_MS = 700;

/**
 * The apps the meeting watcher actually recognizes
 * (`src-tauri/core/src/watch/apps.rs::KNOWN_APPS`) — ids and display names
 * copied from there so `Settings.autoRecord` keys line up with what the
 * watcher writes. Google Meet is deliberately not in this table (see the
 * note rendered below the list): a browser being open is not a call, so
 * there is no real signal to build a control on.
 */
const KNOWN_APPS: Array<{ id: string; label: string }> = [
  { id: "zoom", label: "Zoom" },
  { id: "teams", label: "Microsoft Teams" },
  { id: "slack", label: "Slack" },
  { id: "webex", label: "Webex" },
  { id: "discord", label: "Discord" },
  { id: "facetime", label: "FaceTime" },
];

const POLICY_OPTIONS: Array<{ value: AutoRecordPolicy; label: string }> = [
  { value: "ask", label: "Ask" },
  { value: "always", label: "Always" },
  { value: "never", label: "Never" },
];

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** "Not installed" / "Installed, not running" / … — never red; only a failed pull is an error. */
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
  const [inputDevices, setInputDevices] = useState<InputDevice[]>([]);
  // `null` until the OS has answered, and permanently null off the desktop —
  // which is what keeps the row out of the served UI, where there is no login
  // item to set. See `getAutostart`.
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

  const [storageDraft, setStorageDraft] = useState("");
  const [baseUrlDraft, setBaseUrlDraft] = useState("");
  const [modelDraft, setModelDraft] = useState("");

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
        // Seed the text-field drafts in the same update as `settings` itself
        // (rather than a follow-up effect keyed on `settings?.field`) so the
        // form shows real values on the very first render that has data —
        // no extra render cycle where a control briefly reads empty.
        setSettings(loaded ?? null);
        if (loaded) {
          setStorageDraft(loaded.storageRoot);
          setBaseUrlDraft(loaded.llmBaseUrl);
          setModelDraft(loaded.llmModel);
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
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    listInputDevices().then((devices) => {
      if (!cancelled) setInputDevices(devices);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Read on every open rather than kept anywhere: the login item belongs to the
  // OS, and someone can turn it off in Windows' own Startup Apps list without
  // this app ever hearing about it. Asking is the only way to be right.
  useEffect(() => {
    let cancelled = false;
    getAutostart().then((on) => {
      if (!cancelled) setAutostartState(on);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const updateSettings = useCallback(
    async (next: SettingsData) => {
      // Optimistic: the control shows the new value immediately, which is
      // also what "reflects the round trip" means for a fire-and-forget
      // command with no return payload to reconcile against.
      setSettings(next);
      try {
        await api.setSettings(next);
        // Only after it landed. Telling App a write succeeded when it did not
        // would have it refetch and register the old accelerator as the new one.
        onSaved?.();
      } catch (err) {
        setLoadError(describeError(err));
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
    refreshPullProgress();
    pullTimer.current = setInterval(refreshPullProgress, PULL_POLL_MS);
    return () => {
      if (pullTimer.current) {
        clearInterval(pullTimer.current);
        pullTimer.current = null;
      }
    };
  }, [pulling, refreshPullProgress]);

  const pullEntry = settings ? progress.find((p) => p.name === settings.llmModel) : undefined;

  useEffect(() => {
    if (pullEntry?.done) {
      setPulling(false);
      // The pull finished (success or failure) — re-check status so a
      // successful pull flips "modelReady" without waiting for a poll tick.
      api
        .ollamaStatus()
        .then((s) => setOllama(s ?? null))
        .catch((err) => setLoadError(describeError(err)));
    }
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
      const next = await api.setupStatus();
      setSetup(next ?? null);
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
      if (result.kind === "available") {
        setUpdate(result.update);
      } else if (result.kind === "current") {
        setUpdateMessage("You’re up to date.");
      } else {
        setUpdateMessage("Updates are available in the installed desktop app.");
      }
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
      // `relaunch` normally closes this window. This is a useful fallback if
      // the platform takes a moment before it exits.
      setUpdateMessage("Update installed. Restarting Notetaker…");
    } catch (err) {
      setUpdateMessage(`Couldn’t install the update: ${describeError(err)}`);
      setInstallingUpdate(false);
    }
  }

  async function chooseStorageFolder() {
    const dir = await pickFolder();
    if (!dir || !settings) return;
    // The text box is a draft seeded when settings loaded, so it has to move
    // too. Otherwise it keeps showing the old path — and the next time it
    // loses focus, `commitStorage` writes that old path straight back over
    // the folder just chosen.
    setStorageDraft(dir);
    updateSettings({ ...settings, storageRoot: dir });
  }

  function commitStorage() {
    if (!settings) return;
    const trimmed = storageDraft.trim();
    if (trimmed && trimmed !== settings.storageRoot) {
      updateSettings({ ...settings, storageRoot: trimmed });
    } else {
      setStorageDraft(settings.storageRoot);
    }
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
    if (trimmed && trimmed !== settings.llmBaseUrl) {
      updateSettings({ ...settings, llmBaseUrl: trimmed });
    } else {
      setBaseUrlDraft(settings.llmBaseUrl);
    }
  }

  function commitModel() {
    if (!settings) return;
    const trimmed = modelDraft.trim();
    if (trimmed && trimmed !== settings.llmModel) {
      updateSettings({ ...settings, llmModel: trimmed });
    } else {
      setModelDraft(settings.llmModel);
    }
  }

  function handleAutoRecordChange(appId: string, policy: AutoRecordPolicy) {
    if (!settings) return;
    updateSettings({
      ...settings,
      autoRecord: { ...settings.autoRecord, [appId]: policy },
    });
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
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href]'
      )
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
          <h2 id="settings-title">Settings</h2>
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

        <div className="grid min-h-0 flex-1 grid-cols-[168px_1fr]">
          <nav aria-label="Settings sections" className="settings-nav">
            {SECTIONS.map((s) => (
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
          </nav>

          <div className="settings-panel__body">
            {loadError && <Notice className="mb-3">{loadError}</Notice>}

            {section === "updates" ? (
              <section aria-labelledby="settings-heading-updates">
                <h3 id="settings-heading-updates" className="mb-3 text-[15px] font-semibold text-fg">
                  Updates
                </h3>
                <p className="settings-hint">
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
                            Math.round((updateProgress.received / updateProgress.total) * 100)
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
                  <section aria-labelledby="settings-heading-general">
                    <h3 id="settings-heading-general" className="mb-3 text-[15px] font-semibold text-fg">
                      General
                    </h3>

                    <div className="settings-field">
                      <label htmlFor="settings-theme">Theme</label>
                      <select
                        id="settings-theme"
                        value={theme.preference === "system" ? "" : theme.preference}
                        onChange={(e) =>
                          theme.setPreference(
                            e.target.value === "" ? "system" : (e.target.value as ThemePreference)
                          )
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
                        onCheckedChange={(v) => updateSettings({ ...settings, closeToTray: v })}
                        label="Close button hides to tray"
                      />
                      <span className="settings-field__label-text">Close button hides to tray</span>
                    </div>

                    {isDesktop() && autostart !== null && (
                      <div className="settings-field settings-field--checkbox">
                        <Switch
                          checked={autostart}
                          onCheckedChange={(v) => {
                            // Shown state moves first: the OS write is a round
                            // trip, and a switch that lags behind the finger
                            // reads as a broken switch. `getAutostart` on the
                            // next open is what corrects a write that failed.
                            setAutostartState(v);
                            void setAutostart(v);
                          }}
                          label="Start Notetaker with Windows"
                        />
                        <span className="settings-field__label-text">Start Notetaker with Windows</span>
                      </div>
                    )}

                    <p className="settings-hint">
                      Which languages you expect to hear. Chinese, Cantonese, Japanese and Korean use a
                      second speech model that is much more accurate on them; English and everything
                      else use the model that is always installed.
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
                                : current.filter((c) => c !== choice.code);
                              // Never empty: with nothing selected there is no
                              // basis for choosing a model at all.
                              updateSettings({
                                ...settings,
                                languages: next.length === 0 ? ["en"] : next,
                              });
                            }}
                          />
                          <span>{choice.label}</span>
                        </label>
                      ))}
                    </div>
                  </section>
                )}

                {section === "recording" && (
                  <section aria-labelledby="settings-heading-recording">
                    <h3 id="settings-heading-recording" className="mb-3 text-[15px] font-semibold text-fg">
                      Recording
                    </h3>

                    <div className="settings-field">
                      <label htmlFor="settings-input-device">Microphone</label>
                      <select
                        id="settings-input-device"
                        value={settings.inputDevice ?? ""}
                        onChange={(e) =>
                          updateSettings({
                            ...settings,
                            inputDevice: e.target.value === "" ? null : e.target.value,
                          })
                        }
                      >
                        <option value="">System default</option>
                        {settings.inputDevice &&
                          !inputDevices.some((device) => device.id === settings.inputDevice) && (
                            // The saved device isn't in the current list (unplugged,
                            // or this is an older shell without the Task 8 native
                            // enumeration yet). Without this, the <select> falls back
                            // to "System default" — showing the wrong value rather
                            // than the one actually saved.
                            <option value={settings.inputDevice}>{settings.inputDevice}</option>
                          )}
                        {inputDevices.map((device) => (
                          <option key={device.id} value={device.id}>
                            {device.label}
                          </option>
                        ))}
                      </select>
                    </div>

                    {isDesktop() && (
                      <div className="settings-field">
                        <label htmlFor="settings-overlay">Floating overlay</label>
                        <select
                          id="settings-overlay"
                          value={settings.overlay ?? "recording"}
                          onChange={(e) =>
                            updateSettings({
                              ...settings,
                              overlay: e.target.value as OverlayMode,
                            })
                          }
                        >
                          <option value="recording">While recording</option>
                          <option value="meeting">When a meeting is detected</option>
                          <option value="off">Never</option>
                        </select>
                        <p className="settings-hint">
                          The small always-on-top pill with the timer and controls. "When a
                          meeting is detected" also makes it the record-this-meeting? prompt.
                          It never appears in screen shares.
                        </p>
                      </div>
                    )}

                    <p className="settings-hint">
                      When one of these opens, decide what happens: ask every time, always record without
                      asking, or never ask again.
                    </p>
                    {KNOWN_APPS.map((app) => (
                      <fieldset className="settings-app-policy" key={app.id}>
                        <legend>{app.label}</legend>
                        <div className="settings-app-policy__options">
                          {POLICY_OPTIONS.map((opt) => {
                            const id = `autorecord-${app.id}-${opt.value}`;
                            const checked = (settings.autoRecord[app.id] ?? "ask") === opt.value;
                            return (
                              <span key={opt.value} className="settings-app-policy__option">
                                <input
                                  type="radio"
                                  id={id}
                                  name={`autorecord-${app.id}`}
                                  checked={checked}
                                  onChange={() => handleAutoRecordChange(app.id, opt.value)}
                                />
                                <label htmlFor={id}>{opt.label}</label>
                              </span>
                            );
                          })}
                        </div>
                      </fieldset>
                    ))}
                    <p className="settings-hint">
                      Google Meet isn't in this list. A browser being open doesn't mean you're on a call, so we
                      can't reliably detect Meet meetings yet — start that recording yourself when you join one.
                    </p>

                    <div className="settings-field settings-field--checkbox">
                      <input
                        type="checkbox"
                        id="settings-keep-wav"
                        checked={settings.keepWav}
                        onChange={(e) => updateSettings({ ...settings, keepWav: e.target.checked })}
                      />
                      <label htmlFor="settings-keep-wav">Keep the original recording file too</label>
                    </div>
                    <p className="settings-hint">
                      After we compress a finished recording losslessly (no quality lost), we normally delete
                      the in-between copy. Turning this on keeps it — the exact same audio, just using about
                      twice the disk space.
                    </p>

                    <div className="settings-field settings-field--checkbox">
                      <input
                        type="checkbox"
                        id="settings-process-when-idle"
                        checked={settings.processWhenIdle}
                        onChange={(e) => updateSettings({ ...settings, processWhenIdle: e.target.checked })}
                      />
                      <label htmlFor="settings-process-when-idle">Wait until I'm not using the computer</label>
                    </div>
                    <div className="settings-field">
                      <label htmlFor="settings-min-idle-minutes">Minutes of inactivity before starting</label>
                      <p className="settings-hint">
                        Only matters when the option above is on — we wait this long after you stop using the
                        computer so processing never interrupts you.
                      </p>
                      <input
                        id="settings-min-idle-minutes"
                        type="number"
                        min={1}
                        step={1}
                        value={Math.max(1, Math.round(settings.minIdleSecs / 60))}
                        onChange={(e) => {
                          const minutes = Number(e.target.value);
                          if (!Number.isFinite(minutes) || minutes <= 0) return;
                          updateSettings({ ...settings, minIdleSecs: Math.round(minutes * 60) });
                        }}
                      />
                    </div>
                    <div className="settings-field settings-field--checkbox">
                      <input
                        type="checkbox"
                        id="settings-require-ac"
                        checked={settings.requireAc}
                        onChange={(e) => updateSettings({ ...settings, requireAc: e.target.checked })}
                      />
                      <label htmlFor="settings-require-ac">Only process while plugged in</label>
                    </div>
                  </section>
                )}

                {section === "hotkeys" && (
                  <section aria-labelledby="settings-heading-hotkeys">
                    <h3 id="settings-heading-hotkeys" className="mb-3 text-[15px] font-semibold text-fg">
                      Hotkeys
                    </h3>

                    <div className="flex flex-col gap-2">
                      <HotkeyField
                        label="Start / stop recording"
                        hint="Works anywhere, even with the window closed"
                        value={settings.hotkeyToggleRecord}
                        issue={hotkeyIssues?.toggleRecord ?? null}
                        onChange={(a) => updateSettings({ ...settings, hotkeyToggleRecord: a })}
                      />
                      <HotkeyField
                        label="Show / hide Notetaker"
                        hint="Brings the window up from the tray"
                        value={settings.hotkeyShowHide}
                        issue={hotkeyIssues?.showHide ?? null}
                        onChange={(a) => updateSettings({ ...settings, hotkeyShowHide: a })}
                      />
                      <HotkeyField
                        label="Star this moment"
                        hint="Bookmarks the current moment of a live recording — the AI treats starred moments as important"
                        value={settings.hotkeyHighlight}
                        issue={hotkeyIssues?.highlight ?? null}
                        onChange={(a) => updateSettings({ ...settings, hotkeyHighlight: a })}
                      />
                    </div>
                  </section>
                )}

                {section === "ai" && (
                  <section aria-labelledby="settings-heading-ai">
                    <h3 id="settings-heading-ai" className="mb-3 text-[15px] font-semibold text-fg">
                      Transcription & AI
                    </h3>

                    <div className="settings-field">
                      <label htmlFor="settings-tier-override">Detected: {detectedTier ?? "checking…"}</label>
                      <p className="settings-hint">
                        This controls how big and accurate the on-device transcription and summary models are —
                        bigger is more accurate but slower. The app already picked based on this computer's hardware;
                        only change it if you want something different.
                      </p>
                      <select
                        id="settings-tier-override"
                        value={settings.tierOverride ?? ""}
                        onChange={(e) =>
                          updateSettings({
                            ...settings,
                            tierOverride: e.target.value === "" ? null : e.target.value,
                          })
                        }
                      >
                        <option value="">Use the detected size</option>
                        <option value="small">Small — fastest, least accurate</option>
                        <option value="medium">Medium — balanced</option>
                        <option value="large">Large — most accurate, needs a powerful computer</option>
                      </select>
                    </div>

                    <div className="settings-field">
                      <label htmlFor="settings-speech-engine">Speech model</label>
                      <select
                        id="settings-speech-engine"
                        value={settings.speechEngine ?? "auto"}
                        onChange={(e) =>
                          updateSettings({
                            ...settings,
                            speechEngine: e.target.value as SpeechEngine,
                          })
                        }
                      >
                        <option value="auto">Automatic — pick per sentence</option>
                        <option value="whisper">Always use Whisper</option>
                        <option value="senseVoice">Always use SenseVoice</option>
                      </select>
                    </div>
                    <p className="settings-hint">
                      Automatic works out the language of each thing that gets said and uses whichever
                      model is better at it, so a conversation that switches language mid-sentence still
                      comes out right. Change this only if a transcript comes out wrong.
                    </p>

                    {setup && setup.missing.length > 0 && (
                      <div className="settings-field">
                        {setup.missing.map((model) => (
                          <p key={model.name} className="settings-hint">
                            {model.label} ({formatBytes(model.bytes)})
                          </p>
                        ))}
                        <Button size="sm" onClick={() => void handleDownloadModels()} disabled={downloadingModels}>
                          Download
                        </Button>
                      </div>
                    )}

                    <div className="settings-field">
                      <span id="settings-ollama-heading" className="settings-field__label-text">
                        Local AI (Ollama)
                      </span>
                      <p className="settings-hint">
                        Ollama runs the small AI model that writes your summaries, entirely on this computer.
                      </p>
                      <p className="settings-status">
                        <span className={`status-chip status-chip--${ollamaStatusKind(ollama)}`}>
                          <span className="status-chip__dot" aria-hidden="true" />
                          {ollamaStatusLabel(ollama)}
                        </span>
                      </p>
                      {ollama?.installHint && (
                        <p className="settings-hint settings-hint--action">{ollama.installHint}</p>
                      )}
                      {ollama?.running && (
                        <div className="settings-pull">
                          <button type="button" onClick={handlePull} disabled={pulling}>
                            {ollama.modelReady ? "Pull again" : "Pull model"}
                          </button>
                          {(pulling || pullEntry?.error) && (
                            <PullBar entry={pullEntry} fallbackName={settings.llmModel} />
                          )}
                        </div>
                      )}
                    </div>

                    <details>
                      <summary className="settings-field__label-text">Advanced</summary>
                      <div className="settings-field">
                        <label htmlFor="settings-llm-base-url">AI service address</label>
                        <p className="settings-hint">
                          Where the app looks for the local AI that writes your summaries. Leave this as-is unless
                          you're running something other than the default.
                        </p>
                        <input
                          id="settings-llm-base-url"
                          type="text"
                          value={baseUrlDraft}
                          onChange={(e) => setBaseUrlDraft(e.target.value)}
                          onBlur={commitBaseUrl}
                        />
                      </div>
                      <div className="settings-field">
                        <label htmlFor="settings-llm-model">Summary AI model</label>
                        <p className="settings-hint">Which model on that service writes your summaries.</p>
                        <input
                          id="settings-llm-model"
                          type="text"
                          value={modelDraft}
                          onChange={(e) => setModelDraft(e.target.value)}
                          onBlur={commitModel}
                        />
                      </div>
                    </details>
                  </section>
                )}

                {section === "storage" && (
                  <section aria-labelledby="settings-heading-storage">
                    <h3 id="settings-heading-storage" className="mb-3 text-[15px] font-semibold text-fg">
                      Storage
                    </h3>
                    <div className="settings-field">
                      <label htmlFor="settings-storage-root">Where recordings are saved</label>
                      <p className="settings-hint">
                        The folder on this computer where your recordings, transcripts, and summaries live.
                      </p>
                      <div className="flex items-center gap-2">
                        <input
                          id="settings-storage-root"
                          type="text"
                          value={storageDraft}
                          onChange={(e) => setStorageDraft(e.target.value)}
                          onBlur={commitStorage}
                        />
                        {isDesktop() && (
                          <Button
                            variant="secondary"
                            size="sm"
                            className="shrink-0"
                            onClick={() => void chooseStorageFolder()}
                          >
                            Choose folder…
                          </Button>
                        )}
                      </div>
                    </div>
                    <div className="settings-field">
                      <button type="button" onClick={() => void openLogFolder()}>
                        Open the log folder
                      </button>
                      <p className="settings-hint">If something goes wrong, this is what to send.</p>
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
