import { useEffect } from "react";
import { checkForUpdate, installUpdate } from "../lib/updater";

const STARTUP_DELAY_MS = 30_000;
const RECHECK_MS = 6 * 60 * 60 * 1_000;

/**
 * Checks and installs a signed update without making someone hunt through
 * Settings. The caller owns the safety check so a live recording is never
 * interrupted by a relaunch.
 */
export function useAutoUpdate(safeToRestart: boolean) {
  useEffect(() => {
    if (!safeToRestart) return;

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const checkAndInstall = async () => {
      try {
        const result = await checkForUpdate();
        if (cancelled || result.kind !== "available") return;
        await installUpdate(result.update, () => {});
      } catch {
        // A manual check in Settings can explain a problem. Automatic updates
        // must never interrupt the app just because the release endpoint is
        // temporarily unreachable.
      } finally {
        if (!cancelled) timer = setTimeout(checkAndInstall, RECHECK_MS);
      }
    };

    timer = setTimeout(checkAndInstall, STARTUP_DELAY_MS);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [safeToRestart]);
}
