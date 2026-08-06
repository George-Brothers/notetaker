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
  highlight: string | null;
  dictation: string | null;
}

type DictationMode = "pushToTalk" | "toggle";

/**
 * Every field of `HotkeyIssues`, tied to the type rather than copied from it.
 *
 * `Record<keyof HotkeyIssues, true>` cannot be satisfied unless every key is
 * present, so adding a third issue to the interface fails to compile *here*
 * instead of being silently dropped by the comparison below — which would
 * quietly stop the new issue from ever reaching the panel.
 */
const ISSUE_FIELDS = Object.keys({
  toggleRecord: true,
  showHide: true,
  highlight: true,
  dictation: true,
} satisfies Record<keyof HotkeyIssues, true>) as Array<keyof HotkeyIssues>;

function sameIssues(a: HotkeyIssues, b: HotkeyIssues): boolean {
  return ISSUE_FIELDS.every((field) => a[field] === b[field]);
}

/**
 * The shortcut plugin, loaded once and kept.
 *
 * Dynamic, because this is desktop-only and a static import would put it in
 * the served web bundle.
 *
 * The memo is a tidy-up, not a bug fix — ESM already caches a dynamic import by
 * specifier, so the bare `import()` this replaced was resolving to the same
 * namespace object every time in a real browser. What it buys is one round trip
 * instead of one per registration, and determinism under vitest, where a mocked
 * module can otherwise resolve twice and hand back two different objects.
 *
 * A rejection is deliberately *not* kept: caching it would mean the hotkeys
 * could never recover, where the un-memoised version simply retried on the next
 * effect run. Clearing the slot preserves that.
 */
let shortcutsPlugin: Promise<typeof import("@tauri-apps/plugin-global-shortcut")> | null = null;
function shortcuts() {
  shortcutsPlugin ??= import("@tauri-apps/plugin-global-shortcut").catch((err) => {
    shortcutsPlugin = null;
    throw err;
  });
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
  highlight,
  onToggleRecord,
  onHighlight,
  dictationHotkey,
  dictationMode = "pushToTalk",
  dictating = false,
  onDictationStart,
  onDictationStop,
  onDictationToggle,
  onDictationCancel,
}: {
  enabled: boolean;
  toggleRecord: string;
  showHide: string;
  highlight: string;
  onToggleRecord: () => void;
  /** Stars the current moment; identity-stable for the same reason as onToggleRecord. */
  onHighlight: () => void;
  /** Optional until the dictation settings are available to a caller. */
  dictationHotkey?: string;
  dictationMode?: DictationMode;
  dictating?: boolean;
  onDictationStart?: () => void;
  onDictationStop?: () => void;
  onDictationToggle?: () => void;
  onDictationCancel?: () => void;
}): { issues: HotkeyIssues } {
  const [issues, setIssues] = useState<HotkeyIssues>({
    toggleRecord: null,
    showHide: null,
    highlight: null,
    dictation: null,
  });

  useEffect(() => {
    if (!enabled || !isDesktop()) return;
    let cancelled = false;

    void (async () => {
      const { register, unregisterAll } = await shortcuts();
      await unregisterAll().catch(() => undefined);
      if (cancelled) return;

      const next: HotkeyIssues = {
        toggleRecord: null,
        showHide: null,
        highlight: null,
        dictation: null,
      };

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

      if (!isSafeAccelerator(highlight)) {
        next.highlight = UNSAFE_COPY;
      } else {
        try {
          await register(highlight, (e) => {
            if (e.state === "Pressed") onHighlight();
          });
        } catch {
          next.highlight = CONFLICT_COPY;
        }
      }

      if (dictationHotkey) {
        if (!isSafeAccelerator(dictationHotkey)) {
          next.dictation = UNSAFE_COPY;
        } else {
          try {
            await register(dictationHotkey, (e) => {
              if (dictationMode === "toggle") {
                if (e.state === "Pressed") onDictationToggle?.();
                return;
              }
              if (e.state === "Pressed") onDictationStart?.();
              if (e.state === "Released") onDictationStop?.();
            });
          } catch {
            next.dictation = CONFLICT_COPY;
          }
        }
      }

      // Escape exists only for an active dictation run. Keeping it out of the
      // normal registration set means Escape remains ordinary text input the
      // rest of the time.
      if (dictating) {
        try {
          await register("Escape", (e) => {
            if (e.state === "Pressed") onDictationCancel?.();
          });
        } catch {
          next.dictation ??= "Escape could not be registered; use Stop to end dictation.";
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
      if (!cancelled) setIssues((prev) => (sameIssues(prev, next) ? prev : next));
    })().catch(() => {
      if (!cancelled) {
        setIssues((prev) => ({
          ...prev,
          dictation: dictationHotkey
            ? "Global shortcut service unavailable; dictation hotkey is not active."
            : prev.dictation,
        }));
      }
    });

    return () => {
      cancelled = true;
      void shortcuts()
        .then((m) => m.unregisterAll())
        .catch(() => undefined);
    };
  }, [
    enabled,
    toggleRecord,
    showHide,
    highlight,
    onToggleRecord,
    onHighlight,
    dictationHotkey,
    dictationMode,
    dictating,
    onDictationStart,
    onDictationStop,
    onDictationToggle,
    onDictationCancel,
  ]);

  return { issues };
}
