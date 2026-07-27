import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api } from "../lib/ipc";
import type { OllamaStatus, PullProgress, Settings as SettingsData } from "../lib/ipc";

export interface FirstRunProps {
  onDismiss: () => void;
}

const POLL_MS = 1500;

type ItemStatus = "done" | "in-progress" | "not-started" | "info";

const STATUS_LABEL: Record<ItemStatus, string> = {
  done: "Done",
  "in-progress": "In progress",
  "not-started": "Not started",
  info: "Checked on the Mac",
};

// `PullProgress` has no field distinguishing an Ollama pull from a
// whisper/sherpa speech-model fetch (see ipc.ts) — both share the same
// shape, keyed only by `name`. This is a best-effort classification by name
// until the contract grows a `kind`; reported to the runtime owner.
const SPEECH_MODEL_HINTS = ["whisper", "sense", "sherpa", "diariz", "pyannote"];

function looksLikeSpeechModel(name: string): boolean {
  const lower = name.toLowerCase();
  return SPEECH_MODEL_HINTS.some((hint) => lower.includes(hint));
}

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
 * The first-run checklist (spec: permissions, speech models, Ollama).
 * Deliberately not a wizard: it renders as a dismissible card, not a
 * blocking overlay, because recording works with none of these three items
 * done — the record bar next to it stays fully usable the whole time.
 */
export function FirstRun({ onDismiss }: FirstRunProps) {
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [progress, setProgress] = useState<PullProgress[]>([]);
  const [pulling, setPulling] = useState(false);
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
      const next = await api.pullProgress();
      setProgress(next ?? []);
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

  const speechEntries = progress.filter((p) => looksLikeSpeechModel(p.name));
  const speechStatus: ItemStatus =
    speechEntries.length === 0
      ? "not-started"
      : speechEntries.every((p) => p.done && !p.error)
        ? "done"
        : "in-progress";

  const ollamaEntry = settings ? progress.find((p) => p.name === settings.llmModel) : undefined;

  const ollamaItemStatus: ItemStatus = !ollama
    ? "not-started"
    : !ollama.installed
      ? "not-started"
      : !ollama.modelReady
        ? "in-progress"
        : "done";

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
        <ChecklistItem index={1} title="Microphone and screen-recording permissions" status="info">
          <p className="first-run__item-hint">
            macOS asks for these the first time you record — nothing to check from here.
          </p>
        </ChecklistItem>

        <ChecklistItem index={2} title="Download the speech models" status={speechStatus}>
          {speechEntries.length === 0 ? (
            <p className="first-run__item-hint">Happens automatically in the background — nothing to do yet.</p>
          ) : (
            speechEntries.map((entry) => <ProgressRow key={entry.name} entry={entry} />)
          )}
        </ChecklistItem>

        <ChecklistItem index={3} title="Install Ollama and download the summary model" status={ollamaItemStatus}>
          {ollama && !ollama.installed && ollama.installHint && (
            <p className="first-run__item-hint">{ollama.installHint}</p>
          )}
          {ollama?.installed && !ollama.modelReady && settings && (
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
