/**
 * Desktop-shell-only commands. These are #[tauri::command]s on the app crate,
 * NOT part of runtime::COMMANDS — the LAN/web build must never call them,
 * which is why every function here checks isDesktop() itself.
 */
import { invoke } from "@tauri-apps/api/core";
import { isDesktop } from "./transport";

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
