import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/ipc";
import type { QueueSnapshot } from "../lib/ipc";

const POLL_MS = 1200;

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Owns the small, frequently changing queue surface without blocking the library. */
export function useProcessingQueue() {
  const [snapshot, setSnapshot] = useState<QueueSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSnapshot(await api.queueSnapshot());
      setError(null);
    } catch (err) {
      setError(describeError(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const run = useCallback(
    async (action: (id: string) => Promise<void>, id: string) => {
      try {
        await action(id);
        await refresh();
      } catch (err) {
        setError(describeError(err));
      }
    },
    [refresh],
  );

  return {
    snapshot,
    error,
    refresh,
    pause: (id: string) => run(api.pauseProcessing, id),
    resume: (id: string) => run(api.resumeProcessing, id),
    cancel: (id: string) => run(api.cancelProcessing, id),
    retry: (id: string) => run(api.retryProcessing, id),
  };
}
