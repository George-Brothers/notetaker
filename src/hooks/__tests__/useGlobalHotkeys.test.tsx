import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const register = vi.fn();
const unregisterAll = vi.fn();
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: (...a: unknown[]) => register(...a),
  unregisterAll: (...a: unknown[]) => unregisterAll(...a),
}));

/** The one window, as the show/hide shortcut sees it. */
const win = {
  isVisible: vi.fn(),
  hide: vi.fn(),
  show: vi.fn(),
  unminimize: vi.fn(),
  setFocus: vi.fn(),
};
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));
vi.mock("../../lib/transport", async (orig) => ({
  ...(await orig()),
  isDesktop: () => true,
}));

import { useGlobalHotkeys } from "../useGlobalHotkeys";

const CONFLICT_COPY = "That combination is taken by another app — pick a different one.";
const UNSAFE_COPY = "That combination needs Ctrl or Alt — pick a different one.";

/** The two accelerators the app ships with, unless Settings says otherwise. */
const DEFAULTS = {
  toggleRecord: "CommandOrControl+Alt+N",
  showHide: "CommandOrControl+Alt+Space",
};

describe("useGlobalHotkeys", () => {
  beforeEach(() => {
    register.mockReset().mockResolvedValue(undefined);
    unregisterAll.mockReset().mockResolvedValue(undefined);
    for (const fn of Object.values(win)) fn.mockReset().mockResolvedValue(undefined);
  });

  // This project runs vitest without `globals`, so React Testing Library's
  // automatic cleanup never installs itself (it looks for a global afterEach).
  // Without this, a hook from an earlier test stays mounted and its in-flight
  // registration lands in the *next* test's mocks — which is exactly what it
  // did, intermittently, before this line existed.
  afterEach(cleanup);

  it("registers both accelerators", async () => {
    const onToggleRecord = vi.fn();
    renderHook(() => useGlobalHotkeys({ enabled: true, ...DEFAULTS, onToggleRecord }));

    await waitFor(() => expect(register).toHaveBeenCalledTimes(2));
    expect(register.mock.calls.map((c) => c[0])).toEqual([
      "CommandOrControl+Alt+N",
      "CommandOrControl+Alt+Space",
    ]);
  });

  it("surfaces a registration failure as the spec's copy", async () => {
    const onToggleRecord = vi.fn();
    register.mockRejectedValueOnce(new Error("already registered"));
    const { result } = renderHook(() =>
      useGlobalHotkeys({ enabled: true, ...DEFAULTS, onToggleRecord }),
    );

    await waitFor(() => expect(result.current.issues.toggleRecord).toBe(CONFLICT_COPY));
    expect(result.current.issues.showHide).toBeNull();
  });

  /**
   * The accelerator does not have to have come from the capture field.
   * `settings.json` predates the "needs Ctrl or Alt" rule and can be edited by
   * hand, so a stored `Shift+N` reaches this hook intact — and registering it
   * OS-wide would swallow every capital N typed on the computer. It is refused
   * here, with its own message: "taken by another app" would be a lie.
   */
  it("refuses to register a stored accelerator with no Ctrl or Alt in it", async () => {
    const onToggleRecord = vi.fn();
    const { result } = renderHook(() =>
      useGlobalHotkeys({
        enabled: true,
        toggleRecord: "Shift+N",
        showHide: "CommandOrControl+Alt+Space",
        onToggleRecord,
      }),
    );

    await waitFor(() => expect(result.current.issues.toggleRecord).toBe(UNSAFE_COPY));
    expect(register.mock.calls.map((c) => c[0])).toEqual(["CommandOrControl+Alt+Space"]);
    expect(result.current.issues.showHide).toBeNull();
  });

  it("does nothing at all until it is enabled", async () => {
    const onToggleRecord = vi.fn();
    const { rerender } = renderHook(
      ({ enabled }: { enabled: boolean }) =>
        useGlobalHotkeys({ enabled, ...DEFAULTS, onToggleRecord }),
      { initialProps: { enabled: false } },
    );

    await waitFor(() => expect(unregisterAll).not.toHaveBeenCalled());
    expect(register).not.toHaveBeenCalled();

    rerender({ enabled: true });
    await waitFor(() => expect(register).toHaveBeenCalledTimes(2));
  });

  it("runs the record callback on press, and not on release", async () => {
    const onToggleRecord = vi.fn();
    renderHook(() => useGlobalHotkeys({ enabled: true, ...DEFAULTS, onToggleRecord }));
    await waitFor(() => expect(register).toHaveBeenCalledTimes(2));

    const handler = register.mock.calls[0][1] as (e: { state: string }) => void;
    handler({ state: "Released" });
    expect(onToggleRecord).not.toHaveBeenCalled();
    handler({ state: "Pressed" });
    expect(onToggleRecord).toHaveBeenCalledTimes(1);
  });

  /**
   * The show/hide half, which is the reason the shortcut exists at all: the
   * window is usually in the tray when it is pressed. `show()` on its own is
   * not enough — a minimized window counts as shown to the OS — so the raise
   * path has to unminimize and focus as well, and the hide path must not.
   */
  describe("the show/hide shortcut", () => {
    /**
     * Registers, then hands back the handler the second `register` got.
     *
     * `onToggleRecord` is hoisted out of the render callback deliberately: it
     * is one of the effect's deps, so an inline `vi.fn()` would be a new
     * function on every render, and the re-render that `setIssues` causes
     * would re-register forever. (It does — 120,000 registrations a second,
     * measured.) App passes a `useCallback`; a test must not be laxer.
     */
    async function showHideHandler() {
      const onToggleRecord = vi.fn();
      renderHook(() => useGlobalHotkeys({ enabled: true, ...DEFAULTS, onToggleRecord }));
      await waitFor(() => expect(register).toHaveBeenCalledTimes(2));
      return register.mock.calls[1][1] as (e: { state: string }) => void;
    }

    it("hides the window when it is showing", async () => {
      win.isVisible.mockResolvedValue(true);
      const handler = await showHideHandler();

      handler({ state: "Pressed" });

      await waitFor(() => expect(win.hide).toHaveBeenCalledTimes(1));
      expect(win.show).not.toHaveBeenCalled();
      expect(win.setFocus).not.toHaveBeenCalled();
    });

    it("raises, unminimizes and focuses the window when it is hidden", async () => {
      win.isVisible.mockResolvedValue(false);
      const handler = await showHideHandler();

      handler({ state: "Pressed" });

      await waitFor(() => expect(win.setFocus).toHaveBeenCalledTimes(1));
      expect(win.show).toHaveBeenCalledTimes(1);
      expect(win.unminimize).toHaveBeenCalledTimes(1);
      expect(win.hide).not.toHaveBeenCalled();
    });

    it("does nothing on the release half of the press", async () => {
      win.isVisible.mockResolvedValue(true);
      const handler = await showHideHandler();

      handler({ state: "Released" });

      await waitFor(() => expect(register).toHaveBeenCalledTimes(2));
      expect(win.isVisible).not.toHaveBeenCalled();
      expect(win.hide).not.toHaveBeenCalled();
      expect(win.show).not.toHaveBeenCalled();
    });
  });
});
