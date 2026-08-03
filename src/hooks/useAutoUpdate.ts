import { useCallback, useEffect, useRef, useState } from "react";
import { checkForUpdate, installUpdate } from "../lib/updater";
import type { PendingUpdate, UpdateProgress } from "../lib/updater";

const STARTUP_DELAY_MS = 30_000;
const RECHECK_MS = 6 * 60 * 60 * 1_000;

export interface AutoUpdateState {
  update: PendingUpdate | null;
  checking: boolean;
  installing: boolean;
  progress: UpdateProgress | null;
  error: string | null;
  checkNow: () => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * Checks for signed updates in the background and keeps an available update
 * visible until the user chooses to install it. Checking is safe during a
 * recording; installing is not, because the signed update restarts the app.
 */
export function useAutoUpdate(safeToRestart: boolean): AutoUpdateState {
  const [update, setUpdate] = useState<PendingUpdate | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const safeToRestartRef = useRef(safeToRestart);

  useEffect(() => {
    safeToRestartRef.current = safeToRestart;
  }, [safeToRestart]);

  const checkNow = useCallback(async () => {
    setChecking(true);
    setError(null);
    try {
      const result = await checkForUpdate();
      if (result.kind === "available") {
        setUpdate(result.update);
      } else if (result.kind === "current") {
        setUpdate(null);
      }
    } catch (err) {
      setError(describeError(err));
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const scheduleCheck = (delay: number) => {
      timer = setTimeout(async () => {
        if (cancelled) return;
        await checkNow();
        if (!cancelled) scheduleCheck(RECHECK_MS);
      }, delay);
    };

    scheduleCheck(STARTUP_DELAY_MS);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [checkNow]);

  const install = useCallback(async () => {
    if (!update || installing) return;
    if (!safeToRestartRef.current) {
      setError("Stop recording before installing the update.");
      return;
    }

    setInstalling(true);
    setProgress(null);
    setError(null);
    try {
      await installUpdate(update, setProgress);
      setUpdate(null);
    } catch (err) {
      setError(describeError(err));
    } finally {
      setInstalling(false);
    }
  }, [installing, update]);

  const dismiss = useCallback(() => {
    setUpdate(null);
    setError(null);
    setProgress(null);
  }, []);

  return { update, checking, installing, progress, error, checkNow, install, dismiss };
}
