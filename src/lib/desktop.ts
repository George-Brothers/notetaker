/**
 * Desktop-shell-only commands. These are #[tauri::command]s on the app crate,
 * NOT part of runtime::COMMANDS — the LAN/web build must never call them,
 * which is why every function here checks isDesktop() itself.
 */
import { invoke } from "@tauri-apps/api/core";
import { isDesktop } from "./transport";
import type { CaptureState } from "./ipc";

export interface InputDevice {
  id: string;
  label: string;
  isDefault: boolean;
}

export interface PermissionStatus {
  microphone: boolean;
  accessibility: boolean;
  inputMonitoring: boolean;
  inputMonitoringRequired: boolean;
}

export async function listInputDevices(): Promise<InputDevice[]> {
  if (!isDesktop()) return [];
  try {
    return await invoke<InputDevice[]>("list_input_devices");
  } catch {
    // The command lands with the native layer; an older shell answers with
    // an error. "System default" alone is the honest offer either way.
    return [];
  }
}

/**
 * Non-prompting TCC/OS permission state. A null result means this is an old
 * shell or a served browser build, not that a grant is absent; callers show
 * that distinction rather than claiming a permission is denied.
 */
export async function getPermissionStatus(): Promise<PermissionStatus | null> {
  if (!isDesktop()) return null;
  return invoke<PermissionStatus>("permission_status");
}

const SYSTEM_SETTINGS_PANES: Record<"microphone" | "accessibility" | "inputMonitoring", string> = {
  microphone: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
  accessibility: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
  inputMonitoring: "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
};

/** Opens the exact macOS Privacy pane for the card the user is looking at. */
export async function openSystemSettings(
  pane: keyof typeof SYSTEM_SETTINGS_PANES,
): Promise<boolean> {
  if (!isMacDesktop()) return false;
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(SYSTEM_SETTINGS_PANES[pane]);
  return true;
}

/** The three icons the tray actually has. */
export type TrayState = "idle" | "recording" | "paused";

/**
 * Which tray icon a capture state should show.
 *
 * Pure, and separated from the IPC below so it can be tested without a shell.
 * `finishing` deliberately maps to `idle`: capture has already stopped and only
 * the encode is still running, so leaving the red dot up would say "still
 * recording" to someone who just pressed Stop.
 */
export function trayStateFor(state: CaptureState): TrayState {
  if (state === "recording") return "recording";
  if (state === "paused") return "paused";
  return "idle";
}

/**
 * The last state and status line the tray was told to show, so an unchanged
 * pair costs nothing. The line carries elapsed time, so while recording this
 * changes — and sends — once a second; that is the price of a menu that reads
 * "Recording — 12:34" and it is one small IPC.
 *
 * Module-level on purpose: the tray is a single OS-level object, so this cache
 * is per-process, matching what it describes. Nothing reads it — it only ever
 * suppresses a redundant IPC — so a stale value can at worst skip one no-op
 * call, never show the wrong icon.
 */
let lastTray: string | null = null;

export async function setTrayStatus(state: CaptureState, statusLine: string): Promise<void> {
  if (!isDesktop()) return;
  const next = trayStateFor(state);
  const key = `${next}|${statusLine}`;
  if (key === lastTray) return;
  lastTray = key;
  try {
    await invoke("set_tray_status", { state: next, statusLine });
  } catch {
    // An older shell has no tray; the app must not care.
  }
}

// --- the three plugin-backed shell services --------------------------------
//
// All three import their plugin dynamically rather than at the top of this
// file. `desktop.ts` is imported by the served web bundle too — `trayStateFor`
// is pure and `isDesktop()` returns false for the rest — and a static import
// would pull three desktop-only plugins into a bundle that can never call
// them. Every one of them still checks `isDesktop()` first, so the browser
// path returns before the import is even reached.

/**
 * A real folder chooser for the storage root.
 *
 * `null` covers both "not on the desktop" and "the person cancelled", which is
 * the same instruction to the caller either way: change nothing.
 */
export async function pickFolder(): Promise<string | null> {
  if (!isDesktop()) return null;
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  } catch {
    return null;
  }
}

/**
 * Whether the OS is set to launch Notetaker at login.
 *
 * `null` means "there is nothing to ask" — a browser, or a shell built before
 * the autostart plugin existed — and is what tells Settings to leave the row
 * out entirely rather than show a switch that cannot answer for itself.
 */
export async function getAutostart(): Promise<boolean | null> {
  if (!isDesktop()) return null;
  try {
    const { isEnabled } = await import("@tauri-apps/plugin-autostart");
    return await isEnabled();
  } catch {
    return null;
  }
}

/**
 * Turns the login item on or off.
 *
 * Answers whether the write actually landed — `false` both off the desktop and
 * when the plugin refused. The switch in Settings ignores that (its shown state
 * is corrected by `getAutostart` on the next open), but the first-run default
 * has to know: it ticks itself off in `localStorage` and never runs again, so
 * marking a refused write as done would leave autostart off permanently.
 */
export async function setAutostart(on: boolean): Promise<boolean> {
  if (!isDesktop()) return false;
  try {
    const { enable, disable } = await import("@tauri-apps/plugin-autostart");
    if (on) await enable();
    else await disable();
    return true;
  } catch {
    // Shown state re-reads on next open; a failed write is visible then.
    return false;
  }
}

/**
 * The desktop shell on macOS specifically.
 *
 * There the window keeps its native titlebar — transparent, overlaid on the
 * app's own header, with the system traffic lights top-left — so anything
 * that exists to *replace* OS chrome must stand down, and the header makes
 * room on its left instead of its right.
 *
 * UA sniffing is safe in this one spot: inside the Tauri shell the webview is
 * WKWebView, whose user agent always carries "Mac OS X", and the served
 * browser build never gets this far because `isDesktop()` is false there.
 * (jsdom's UA says "darwin", never "Mac", so tests opt in explicitly.)
 */
export function isMacDesktop(): boolean {
  return isDesktop() && navigator.userAgent.includes("Mac");
}
