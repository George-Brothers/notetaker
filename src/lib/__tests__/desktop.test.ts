import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listInputDevices, setTrayStatus, trayStateFor } from "../desktop";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("trayStateFor", () => {
  it("maps capture states onto the three tray icons", () => {
    expect(trayStateFor("idle")).toBe("idle");
    expect(trayStateFor("recording")).toBe("recording");
    expect(trayStateFor("paused")).toBe("paused");
    // Finishing is not capturing: the red dot must not linger after stop.
    expect(trayStateFor("finishing")).toBe("idle");
  });
});

describe("the shell-only commands off the desktop", () => {
  /**
   * These two are `#[tauri::command]`s on the app crate and are not in
   * `runtime::COMMANDS`, so the served UI has nothing to answer them. Reaching
   * for them from a browser would throw where the caller expects a value —
   * hence the `isDesktop()` check inside each, which this pins.
   */
  it("never invokes anything, and still answers", async () => {
    await expect(setTrayStatus("recording")).resolves.toBeUndefined();
    await expect(listInputDevices()).resolves.toEqual([]);
    expect(invoke).not.toHaveBeenCalled();
  });
});
