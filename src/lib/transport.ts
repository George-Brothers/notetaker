/**
 * How the UI reaches the Rust core — Tauri IPC on the desktop, HTTP in a
 * browser.
 *
 * `ipc.ts` calls `invoke<T>(name, args)` twenty-four times and does not care
 * which of the two it is talking to. That works because both transports speak
 * exactly the same contract: a command name from `runtime::COMMANDS` and an
 * object of camelCase arguments. On the desktop that goes to a
 * `#[tauri::command]`; in a browser it becomes `POST /api/<name>` handled by
 * `notetaker-server`, which forwards it to the same `notetaker_core::dispatch`
 * entry point. Identical arguments in, identical JSON out.
 *
 * Keeping the switch here rather than in `ipc.ts` means the contract file stays
 * a plain list of commands — which matters, because a Rust test greps it to
 * check the two sides agree.
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** Where the LAN access code is kept once we've seen it. */
const TOKEN_KEY = "notetaker.token";

/**
 * True when running inside the desktop shell.
 *
 * Tauri 2 puts `__TAURI_INTERNALS__` on `window`. Checking for that rather than
 * for the absence of a browser, because the UI runs in a real browser engine
 * either way — there is no other reliable way to tell the two apart.
 */
export function isDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * The LAN access code, if this page needs one.
 *
 * Arrives as `?token=...` the first time — that is the link you open on a phone
 * — and is then kept in `sessionStorage` so moving around the app doesn't lose
 * it. `sessionStorage` rather than `localStorage` deliberately: it dies with the
 * tab, so a shared or borrowed device does not keep the code indefinitely.
 */
export function accessToken(): string | null {
  if (typeof window === "undefined") return null;

  const fromUrl = new URLSearchParams(window.location.search).get("token");
  if (fromUrl) {
    try {
      window.sessionStorage.setItem(TOKEN_KEY, fromUrl);
    } catch {
      // Private browsing can refuse storage. The token still works for this
      // page load; only navigation would lose it.
    }
    return fromUrl;
  }

  try {
    return window.sessionStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

/** The base URL for API calls — same origin as the served UI. */
function apiBase(): string {
  return "/api";
}

/**
 * Calls a core command.
 *
 * Errors are thrown as `Error` with the message the Rust side wrote. The runtime
 * already phrases its failures for someone who is not an engineer, so nothing is
 * reworded here — a transport that invented its own text would replace a useful
 * message with a generic one.
 */
export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isDesktop()) {
    return tauriInvoke<T>(command, args);
  }

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  const token = accessToken();
  if (token) headers["X-Notetaker-Token"] = token;

  let response: Response;
  try {
    response = await fetch(`${apiBase()}/${command}`, {
      method: "POST",
      headers,
      body: JSON.stringify(args ?? {}),
    });
  } catch {
    // A dead connection is the most common failure for a served UI — the laptop
    // went to sleep, or the wifi moved. Say that, rather than "failed to fetch".
    throw new Error(
      "Notetaker is not responding. The computer it runs on may be asleep or off the network.",
    );
  }

  if (response.status === 401) {
    throw new Error(
      "This link needs the access code shown when you turned on network access.",
    );
  }

  const text = await response.text();
  let payload: unknown = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      throw new Error("Notetaker sent back a reply this app could not read.");
    }
  }

  if (!response.ok) {
    const message =
      payload &&
      typeof payload === "object" &&
      typeof (payload as { error?: unknown }).error === "string"
        ? (payload as { error: string }).error
        : "Something went wrong.";
    throw new Error(message);
  }

  return payload as T;
}
