import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, LANGUAGE_CHOICES } from "../lib/ipc";
import type {
  AutoRecordPolicy,
  OllamaStatus,
  PullProgress,
  Settings as SettingsData,
  SpeechEngine,
} from "../lib/ipc";

export interface SettingsProps {
  onClose: () => void;
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

export function Settings({ onClose }: SettingsProps) {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [detectedTier, setDetectedTier] = useState<string | null>(null);
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);
  const [progress, setProgress] = useState<PullProgress[]>([]);
  const [pulling, setPulling] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

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
    return () => {
      cancelled = true;
    };
  }, []);

  const updateSettings = useCallback(async (next: SettingsData) => {
    // Optimistic: the control shows the new value immediately, which is
    // also what "reflects the round trip" means for a fire-and-forget
    // command with no return payload to reconcile against.
    setSettings(next);
    try {
      await api.setSettings(next);
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

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

        <div className="settings-panel__body">
          {loadError && (
            <p className="settings-hint settings-hint--action" role="alert">
              {loadError}
            </p>
          )}

          {!settings ? (
            <p aria-live="polite">Loading settings…</p>
          ) : (
            <>
              <section className="settings-section" aria-labelledby="settings-storage-heading">
                <h3 id="settings-storage-heading">Storage</h3>
                <div className="settings-field">
                  <label htmlFor="settings-storage-root">Where recordings are saved</label>
                  <p className="settings-hint">
                    The folder on this computer where your recordings, transcripts, and summaries live.
                  </p>
                  <input
                    id="settings-storage-root"
                    type="text"
                    value={storageDraft}
                    onChange={(e) => setStorageDraft(e.target.value)}
                    onBlur={commitStorage}
                  />
                </div>
                <div className="settings-field">
                  <button type="button" onClick={() => void openLogFolder()}>
                    Open the log folder
                  </button>
                  <p className="settings-hint">
                    If something goes wrong, this is what to send.
                  </p>
                </div>
              </section>

              <section className="settings-section" aria-labelledby="settings-ai-heading">
                <h3 id="settings-ai-heading">Summary AI</h3>
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
                  {ollama && !ollama.installed && ollama.installHint && (
                    <p className="settings-hint settings-hint--action">{ollama.installHint}</p>
                  )}
                  {ollama?.installed && (
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
              </section>

              <section className="settings-section" aria-labelledby="settings-tier-heading">
                <h3 id="settings-tier-heading">Model size</h3>
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
              </section>

              <section className="settings-section" aria-labelledby="settings-autorecord-heading">
                <h3 id="settings-autorecord-heading">Auto-record known apps</h3>
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
              </section>

              <section className="settings-section" aria-labelledby="settings-processing-heading">
                <h3 id="settings-processing-heading">When recordings process</h3>
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

              <section className="settings-section" aria-labelledby="settings-speech-heading">
                <h3 id="settings-speech-heading">Languages and speech</h3>
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
              </section>

              <section className="settings-section" aria-labelledby="settings-storage-format-heading">
                <h3 id="settings-storage-format-heading">Recording files</h3>
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
              </section>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
