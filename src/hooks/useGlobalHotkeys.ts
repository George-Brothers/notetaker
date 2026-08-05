/**
 * OS-wide shortcuts. Registered from the webview because the webview owns the
 * capture flow and the settings; the window being hidden does not stop its JS.
 * Failures are surfaced, never silent — a hotkey that quietly does nothing is
 * indistinguishable from a broken app.
 *
 * `onToggleRecord` should be identity-stable (App passes a `useCallback`) — it
 * is one of the registration effect's dependencies. A caller that gets that
 * wrong is no longer catastrophic, though: the `setIssues` at the end of the
 * effect bails out when nothing changed, which breaks the re-render that would
 * otherwise make it an unregister/re-register loop against the OS.
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
 * The shortcut plugin, loaded once and kept.
 *
 * Dynamic, because this is desktop-only and a static import would put it in
 * the served web bundle. Memoised, because *every* call must reach the same
 * module instance: `register` and the `unregisterAll` in the cleanup below
 * are two halves of one piece of OS state, and two instances would mean
 * unregistering shortcuts a different copy of the plugin owns.
 */
let shortcutsPlugin: Promise<typeof import("@tauri-apps/plugin-global-shortcut")> | null = null;
function shortcuts() {
  shortcutsPlugin ??= import("@tauri-apps/plugin-global-shortcut");
  return shortcutsPlugin;
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
      const { register, unregisterAll } = await shortcuts();
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

      // Field by field, and bail out when nothing changed. A fresh object here
      // every time would re-render, and a caller that hands this hook a new
      // `onToggleRecord` each render — one inline arrow where a `useCallback`
      // belongs — would turn that re-render into an unregister/re-register
      // loop against the OS shortcut table. Measured, before this guard
      // existed: 120,952 registrations inside one second, with nothing in the
      // symptom pointing at the callback. Returning `prev` makes React skip
      // the re-render, which is what ends the cycle.
      if (!cancelled) {
        setIssues((prev) =>
          prev.toggleRecord === next.toggleRecord && prev.showHide === next.showHide ? prev : next,
        );
      }
    })();

    return () => {
      cancelled = true;
      void shortcuts()
        .then((m) => m.unregisterAll())
        .catch(() => undefined);
    };
  }, [enabled, toggleRecord, showHide, onToggleRecord]);

  return { issues };
}
