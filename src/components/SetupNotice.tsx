/**
 * One honest line about what the app cannot currently do.
 *
 * The app records perfectly well with nothing downloaded, and this never stops
 * it — no modal, no disabled buttons, no blocking the way in. It exists because
 * the alternative is what actually shipped: a user recorded three meetings,
 * pressed "Process now", and got a success. Nothing had failed. There was
 * simply no transcription running, and no screen in the app knew that or said
 * it.
 *
 * "Not now" is real. It hides this until the app restarts, and everything keeps
 * working exactly as it did.
 */

import { useCallback, useEffect, useState } from "react";
import { Download } from "lucide-react";
import { api, type SetupStatus } from "../lib/ipc";
import { Button, Notice } from "./ui";

/** Bytes as a person would say them: "213 MB", "1.6 GB". */
export function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${Math.round(bytes / 1_000_000)} MB`;
  return `${Math.max(1, Math.round(bytes / 1_000))} KB`;
}

/**
 * What to say, given the state. Split out from the component so the wording —
 * the entire point of this file — can be tested without rendering anything.
 *
 * Returns null when there is nothing worth saying, which is the common case:
 * a set-up app must be silent.
 */
export function setupMessage(status: SetupStatus | null): string | null {
  if (!status || status.transcribing) return null;
  if (status.missing.length === 0) {
    // Everything is on disk and the loop still is not running. That is a real
    // fault rather than an unfinished setup, and saying "download something"
    // would send the user somewhere that cannot help.
    return "Transcription isn't running, even though the models are downloaded. Restarting the app usually fixes this.";
  }

  const size = formatBytes(status.downloadBytes);
  const waiting =
    status.waiting === 0
      ? ""
      : status.waiting === 1
        ? " One recording is waiting."
        : ` ${status.waiting} recordings are waiting.`;
  return `Recording works, but nothing is being transcribed yet — the speech models aren't downloaded (${size}).${waiting}`;
}

export function SetupNotice({
  onOpenSettings,
  onStatus,
}: {
  onOpenSettings?: () => void;
  onStatus?: (status: SetupStatus | null) => void;
}) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [hidden, setHidden] = useState(false);
  const [starting, setStarting] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const next = await api.setupStatus();
      setStatus(next);
      onStatus?.(next);
    } catch {
      // A status check that cannot run is not worth interrupting anyone over.
      setStatus(null);
      onStatus?.(null);
    }
  }, [onStatus]);

  useEffect(() => {
    void refresh();
    // Slow on purpose. This is a background truth, not a progress bar — the
    // download has its own progress in the checklist.
    const timer = window.setInterval(() => void refresh(), 15_000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const message = setupMessage(status);
  if (hidden || message === null) return null;

  const canDownload = (status?.missing.length ?? 0) > 0;

  return (
    <Notice className="mx-3 mt-2 shrink-0">
      <span>{message}</span>{" "}
      {canDownload && (
        <Button
          size="sm"
          variant="primary"
          disabled={starting}
          onClick={async () => {
            setStarting(true);
            try {
              await api.downloadModels();
              onOpenSettings?.();
            } finally {
              setStarting(false);
              void refresh();
            }
          }}
        >
          <Download size={13} />
          {starting ? "Starting…" : "Download them"}
        </Button>
      )}{" "}
      <button
        type="button"
        onClick={() => setHidden(true)}
        className="underline underline-offset-2"
      >
        Not now
      </button>
    </Notice>
  );
}
