/**
 * OS-wide shortcuts. Registered from the webview because the webview owns the
 * capture flow and the settings; the window being hidden does not stop its JS.
 * Failures are surfaced, never silent — a hotkey that quietly does nothing is
 * indistinguishable from a broken app.
 */
import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isDesktop } from "../lib/transport";
import { isSafeAccelerator } from "../lib/hotkeys";

const CONFLICT_COPY = "That combination is taken by another app — pick a different one.";
/**
 * Deliberately *not* the conflict copy: nothing else has taken `Shift+N`, and
 * saying so would send someone hunting for an app that isn't there. What is
 * wrong is the combination itself — see `isSafeAccelerator`.
 */
const UNSAFE_COPY = "That combination needs Ctrl or Alt — pick a different one.";

export interface HotkeyIssues {
  toggleRecord: string | null;
  showHide: string | null;
}

/**
 * Show/hide, for a window that may be sitting in the tray.
 *
 * `show` alone is not enough on Windows: a minimized window is still "visible"
 * as far as the OS is concerned, so it has to be un-minimized and focused too.
 */
async function toggleWindowVisibility(): Promise<void> {
  const win = getCurrentWindow();
  if (await win.isVisible()) {
    await win.hide();
  } else {
    await win.show();
    await win.unminimize();
    await win.setFocus();
  }
}

export function useGlobalHotkeys({
  enabled,
  toggleRecord,
  showHide,
  onToggleRecord,
}: {
  enabled: boolean;
  toggleRecord: string;
  showHide: string;
  onToggleRecord: () => void;
}): { issues: HotkeyIssues } {
  const [issues, setIssues] = useState<HotkeyIssues>({ toggleRecord: null, showHide: null });

  useEffect(() => {
    if (!enabled || !isDesktop()) return;
    let cancelled = false;

    (async () => {
      const { register, unregisterAll } = await import("@tauri-apps/plugin-global-shortcut");
      await unregisterAll().catch(() => undefined);
      if (cancelled) return;

      const next: HotkeyIssues = { toggleRecord: null, showHide: null };

      // The guard runs here as well as in the capture field because a stored
      // accelerator need not have come from the field: `settings.json` predates
      // this rule and can be edited by hand. Registering `Shift+N` OS-wide
      // would swallow every capital N typed on the computer.
      if (!isSafeAccelerator(toggleRecord)) {
        next.toggleRecord = UNSAFE_COPY;
      } else {
        try {
          await register(toggleRecord, (e) => {
            if (e.state === "Pressed") onToggleRecord();
          });
        } catch {
          next.toggleRecord = CONFLICT_COPY;
        }
      }

      if (!isSafeAccelerator(showHide)) {
        next.showHide = UNSAFE_COPY;
      } else {
        try {
          await register(showHide, (e) => {
            // Nothing useful to say if the window refuses to move: the shortcut
            // registered, so the message here would be about this one press.
            if (e.state === "Pressed") void toggleWindowVisibility().catch(() => undefined);
          });
        } catch {
          next.showHide = CONFLICT_COPY;
        }
      }

      if (!cancelled) setIssues(next);
    })();

    return () => {
      cancelled = true;
      void import("@tauri-apps/plugin-global-shortcut")
        .then((m) => m.unregisterAll())
        .catch(() => undefined);
    };
  }, [enabled, toggleRecord, showHide, onToggleRecord]);

  return { issues };
}
