import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api, LANGUAGE_CHOICES } from "../lib/ipc";
import type { FoundModel, OllamaStatus, PullProgress, Settings as SettingsData, SetupStatus } from "../lib/ipc";
import { getPermissionStatus, isMacDesktop, openSystemSettings } from "../lib/desktop";
import type { PermissionStatus } from "../lib/desktop";

export interface FirstRunProps {
  onDismiss: () => void;
}

const POLL_MS = 1500;

type ItemStatus = "done" | "in-progress" | "not-started" | "info";

const STATUS_LABEL: Record<ItemStatus, string> = {
  done: "Done",
  "in-progress": "In progress",
  "not-started": "Not started",
  info: "Handled by your operating system",
};

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function ProgressRow({ entry }: { entry: PullProgress }) {
  if (entry.error) {
    return (
      <p className="first-run__error" role="alert">
        {entry.name}: {entry.error}
      </p>
    );
  }
  const pct = Math.max(0, Math.min(100, Math.round(entry.percent)));
  return (
    <div className="progress-bar">
      <span className="progress-bar__label">
        {entry.name} — {pct}%
      </span>
      <span
        className="progress-bar__track"
        role="progressbar"
        aria-label={`${entry.name} download progress`}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <span className="progress-bar__fill" style={{ width: `${pct}%` }} />
      </span>
    </div>
  );
}

function ChecklistItem({
  index,
  title,
  status,
  children,
}: {
  index: number;
  title: string;
  status: ItemStatus;
  children?: ReactNode;
}) {
  return (
    <li className={`first-run__item first-run__item--${status}`}>
      <span className="first-run__item-index" aria-hidden="true">
        {index}
      </span>
      <div className="first-run__item-body">
        <div className="first-run__item-head">
          <span className="first-run__item-title">{title}</span>
          <span className="first-run__item-status">{STATUS_LABEL[status]}</span>
        </div>
        {children}
      </div>
    </li>
  );
}

/**
 * Which languages you speak, asked before anything is downloaded.
 *
 * This is a question about *download size*, not a preference. Chinese,
 * Cantonese, Japanese and Korean are the languages a second 239 MB model is
 * markedly better at; English and everything else are handled by the model
 * that gets downloaded regardless. Asking first means a user who only ever
 * speaks English never fetches a model that would never be chosen for their
 * audio.
 */
function LanguageStep({
  selected,
  onChange,
  saving,
}: {
  selected: string[];
  onChange: (next: string[]) => void;
  saving: boolean;
}) {
  function toggle(code: string) {
    const next = selected.includes(code)
      ? selected.filter((c) => c !== code)
      : [...selected, code];
    // Never leave it empty: with nothing chosen there is no basis for deciding
    // what to download, and English is the safe default the app already ships.
    onChange(next.length === 0 ? ["en"] : next);
  }

  const extraModel = LANGUAGE_CHOICES.some(
    (choice) => choice.senseVoice && selected.includes(choice.code),
  );

  return (
    <fieldset className="first-run__languages" disabled={saving}>
      <legend className="first-run__item-hint">
        Pick every language you expect to hear. You can change this later in Settings.
      </legend>
      {LANGUAGE_CHOICES.map((choice) => (
        <label key={choice.code} className="first-run__language">
          <input
            type="checkbox"
            checked={selected.includes(choice.code)}
            onChange={() => toggle(choice.code)}
          />
          <span>{choice.label}</span>
        </label>
      ))}
      <p className="first-run__item-hint">
        {extraModel
          ? "Adds a second speech model (about 239 MB) that is much more accurate on those languages."
          : "Nothing extra to download for these."}
      </p>
    </fieldset>
  );
}

/**
 * The first-run checklist (spec: permissions, speech models, Ollama).
 * Deliberately not a wizard: it renders as a dismissible card, not a
 * blocking overlay, because recording works with none of these three items
 * done — the record bar next to it stays fully usable the whole time.
 */
export function FirstRun({ onDismiss }: FirstRunProps) {
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [permissions, setPermissions] = useState<PermissionStatus | null>(null);
  const [progress, setProgress] = useState<PullProgress[]>([]);
  const [foundModels, setFoundModels] = useState<FoundModel[]>([]);
  const [setup, setSetup] = useState<SetupStatus | null>(null);
  const [pulling, setPulling] = useState(false);
  const [downloadingModels, setDownloadingModels] = useState(false);
  const [adoptingModels, setAdoptingModels] = useState(false);
  const [savingLanguages, setSavingLanguages] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  const refresh = useCallback(async () => {
    try {
      const status = await api.ollamaStatus();
      setOllama(status ?? null);
    } catch (err) {
      setLoadError(describeError(err));
    }
    try {
      setPermissions(await getPermissionStatus());
    } catch (err) {
      // null means the shell could not answer, not that every permission is
      // denied. The rendered card keeps that distinction visible.
      setPermissions(null);
      setLoadError(describeError(err));
    }
    try {
      const next = await api.pullProgress();
      setProgress(next ?? []);
    } catch (err) {
      setLoadError(describeError(err));
    }
    try {
      const next = await api.setupStatus();
      setSetup(next ?? null);
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const loaded = await api.getSettings();
        if (!cancelled) setSettings(loaded ?? null);
      } catch (err) {
        if (!cancelled) setLoadError(describeError(err));
      }
    })();
    api
      .setupStatus()
      .then((status) => {
        if (cancelled) return;
        setSetup(status ?? null);
        // Looking through download folders only helps when models are actually
        // absent. A completed setup used to scan needlessly and, worse, the UI
        // then looked "not started" because it only knew in-memory progress.
        if ((status?.missing.length ?? 0) === 0) return;
        return api.findExistingModels().then((found) => {
          if (!cancelled) setFoundModels(found ?? []);
        });
      })
      .catch((err) => {
        if (!cancelled) setLoadError(describeError(err));
      });
    refresh();
    timer.current = setInterval(refresh, POLL_MS);
    return () => {
      cancelled = true;
      if (timer.current) {
        clearInterval(timer.current);
        timer.current = null;
      }
    };
  }, [refresh]);

  async function handlePull() {
    if (!settings) return;
    setLoadError(null);
    setPulling(true);
    try {
      await api.pullModel(settings.llmModel);
      await refresh();
    } catch (err) {
      setLoadError(describeError(err));
    } finally {
      setPulling(false);
    }
  }

  async function handleLanguages(next: string[]) {
    if (!settings) return;
    const updated = { ...settings, languages: next };
    // Optimistic: the checkbox must feel instant, and a failed write is
    // reported rather than silently reverting the tick the user just made.
    setSettings(updated);
    setLoadError(null);
    setSavingLanguages(true);
    try {
      await api.setSettings(updated);
    } catch (err) {
      setLoadError(describeError(err));
    } finally {
      setSavingLanguages(false);
    }
  }

  async function handleDownloadModels() {
    setLoadError(null);
    setDownloadingModels(true);
    try {
      await api.downloadModels();
      await refresh();
    } catch (err) {
      setLoadError(describeError(err));
    } finally {
      setDownloadingModels(false);
    }
  }

  async function handleAdoptModels() {
    setLoadError(null);
    setAdoptingModels(true);
    try {
      await api.adoptModels();
      await refresh();
    } catch (err) {
      setLoadError(describeError(err));
    } finally {
      setAdoptingModels(false);
    }
  }

  const speechEntries = progress.filter((p) => p.kind === "speech");
  const speechStatus: ItemStatus =
    setup?.missing.length === 0
      ? "done"
      : speechEntries.some((p) => !p.done)
        ? "in-progress"
        : "not-started";

  const ollamaEntry = settings
    ? progress.find((p) => p.kind === "ollama" && p.name === settings.llmModel)
    : undefined;

  const ollamaItemStatus: ItemStatus = !ollama
    ? "not-started"
    : !ollama.installed
      ? "not-started"
      : !ollama.modelReady
        ? "in-progress"
        : "done";

  const macPermissions = isMacDesktop();
  const permissionCheckUnavailable = macPermissions && permissions === null;
  const microphoneStatus: ItemStatus = !macPermissions
    ? "info"
    : permissionCheckUnavailable
      ? "not-started"
      : permissions?.microphone
        ? "done"
        : "in-progress";
  const accessibilityStatus: ItemStatus = !macPermissions
    ? "info"
    : permissionCheckUnavailable || !permissions?.microphone
      ? "not-started"
      : permissions?.accessibility
        ? "done"
        : "in-progress";
  const inputMonitoringVisible =
    macPermissions && permissions?.inputMonitoringRequired === true;
  const inputMonitoringStatus: ItemStatus = !permissions
    ? "not-started"
    : permissions.inputMonitoring
      ? "done"
      : "in-progress";
  const languagesIndex = inputMonitoringVisible ? 4 : macPermissions ? 3 : 2;
  const modelsIndex = languagesIndex + 1;
  const ollamaIndex = modelsIndex + 1;

  async function openPermissionPane(
    pane: "microphone" | "accessibility" | "inputMonitoring",
  ) {
    setLoadError(null);
    try {
      const opened = await openSystemSettings(pane);
      if (!opened) {
        setLoadError("This System Settings link is available from the macOS desktop app.");
      }
    } catch (err) {
      setLoadError(describeError(err));
    }
  }

  return (
    <section className="first-run" role="region" aria-label="Getting started">
      <header className="first-run__header">
        <h2>Getting started</h2>
        <button type="button" className="first-run__dismiss" onClick={onDismiss}>
          Dismiss
        </button>
      </header>
      <p className="first-run__hint">
        You can start recording right now — these just get transcripts and summaries ready in the
        background.
      </p>
      {loadError && (
        <p className="first-run__error" role="alert">
          {loadError}
        </p>
      )}
      <ol className="first-run__list">
        {macPermissions ? (
          <>
            <ChecklistItem index={1} title="Microphone permission" status={microphoneStatus}>
              <p className="first-run__item-hint">
                {permissionCheckUnavailable
                  ? "The desktop shell could not verify this grant. Use the pane below, then return here; this card rechecks automatically."
                  : permissions?.microphone
                    ? "Granted. Dictation will use the selected 16 kHz microphone."
                    : "Allow Notetaker to use the microphone. Dictation cannot produce text until this is granted."}
              </p>
              {!permissions?.microphone && (
                <button type="button" onClick={() => void openPermissionPane("microphone")}>
                  Open Microphone settings
                </button>
              )}
            </ChecklistItem>

            <ChecklistItem index={2} title="Accessibility for automatic paste" status={accessibilityStatus}>
              <p className="first-run__item-hint">
                {!permissions?.microphone
                  ? "Complete the microphone card first."
                  : permissions.accessibility
                    ? "Granted. Notetaker can post the layout-aware Cmd-V event."
                    : "Allow Notetaker under Privacy & Security → Accessibility. Without it, text stays on the clipboard and the app tells you to press Cmd-V."}
              </p>
              <p className="first-run__item-hint">
                Signed identity: <strong>Notetaker Local Signing</strong>. Re-check this row after every rebuild or update; Accessibility persistence across rebuilds is still a hardware verification item.
              </p>
              {permissions?.microphone && !permissions.accessibility && (
                <button type="button" onClick={() => void openPermissionPane("accessibility")}>
                  Open Accessibility settings
                </button>
              )}
            </ChecklistItem>

            {inputMonitoringVisible && (
              <ChecklistItem index={3} title="Input Monitoring (only if required)" status={inputMonitoringStatus}>
                <p className="first-run__item-hint">
                  This feature is not required by the current Carbon shortcut and CGEvent paste path. It appeared because the running shell reported that an event-listening path needs it.
                </p>
                {!permissions.inputMonitoring && (
                  <button type="button" onClick={() => void openPermissionPane("inputMonitoring")}>
                    Open Input Monitoring settings
                  </button>
                )}
              </ChecklistItem>
            )}
          </>
        ) : (
          <ChecklistItem index={1} title="System permissions" status="info">
            <p className="first-run__item-hint">
              Windows does not use these macOS TCC prompts. Dictation reports a concrete microphone or input-injection error if the OS blocks it.
            </p>
          </ChecklistItem>
        )}

        <ChecklistItem
          index={languagesIndex}
          title="Which languages do you speak?"
          status={settings ? "done" : "not-started"}
        >
          {settings && (
            <LanguageStep
              selected={settings.languages ?? ["en"]}
              onChange={handleLanguages}
              saving={savingLanguages}
            />
          )}
        </ChecklistItem>

        <ChecklistItem index={modelsIndex} title="Download the speech models" status={speechStatus}>
          {speechStatus !== "done" && (
            <>
              {foundModels.length > 0 && (
                <div className="first-run__item-hint">
                  <p>Found a copy of this on your computer. Use it instead of downloading?</p>
                  <button type="button" onClick={handleAdoptModels} disabled={adoptingModels}>
                    {adoptingModels ? "Checking it…" : "Use it instead"}
                  </button>
                </div>
              )}
              <button type="button" onClick={handleDownloadModels} disabled={downloadingModels}>
                Download speech models
              </button>
              {speechEntries.length === 0 && (
                <p className="first-run__item-hint">
                  Nothing is transcribed until these are downloaded — about 1.9 GB. It only happens
                  when you press the button.
                </p>
              )}
            </>
          )}
          {speechEntries.map((entry) => (
            <ProgressRow key={entry.name} entry={entry} />
          ))}
        </ChecklistItem>

        <ChecklistItem index={ollamaIndex} title="Install Ollama and download the summary model" status={ollamaItemStatus}>
          {ollama?.installHint && (
            <p className="first-run__item-hint">{ollama.installHint}</p>
          )}
          {ollama?.running && !ollama.modelReady && settings && (
            <>
              <button type="button" onClick={handlePull} disabled={pulling}>
                Pull the summary model
              </button>
              {ollamaEntry && <ProgressRow entry={ollamaEntry} />}
            </>
          )}
        </ChecklistItem>
      </ol>
    </section>
  );
}
