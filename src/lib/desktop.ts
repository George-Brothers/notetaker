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
 * The last icon the tray was told to show, so an unchanged state costs nothing.
 *
 * Module-level on purpose: the tray is a single OS-level object, so this cache
 * is per-process, matching what it describes. Nothing reads it — it only ever
 * suppresses a redundant IPC — so a stale value can at worst skip one no-op
 * call, never show the wrong icon.
 */
let lastTray: TrayState | null = null;

export async function setTrayStatus(state: CaptureState): Promise<void> {
  if (!isDesktop()) return;
  const next = trayStateFor(state);
  if (next === lastTray) return;
  lastTray = next;
  try {
    await invoke("set_tray_status", { state: next });
  } catch {
    // An older shell has no tray; the app must not care.
  }
}
