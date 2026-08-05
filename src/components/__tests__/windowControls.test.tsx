import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WindowControls } from "../WindowControls";
import tauriConf from "../../../src-tauri/tauri.conf.json";

/**
 * The window, faked.
 *
 * `onResized` keeps the handler it was given so a test can play the part of
 * the operating system — dragging to the top edge and Win+Up both arrive as a
 * resize and nothing else, and "does the icon follow a maximise this component
 * did not perform" is the question these controls most easily get wrong.
 *
 * `vi.hoisted` because `vi.mock` factories run before the imports above.
 */
const win = vi.hoisted(() => {
  const resized: Array<() => void> = [];
  const unlisten = vi.fn();
  return {
    resized,
    unlisten,
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async (handler: () => void) => {
      resized.push(handler);
      return unlisten;
    }),
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  };
});

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));

/** What the real Tauri shell puts on `window`, and what `isDesktop()` reads. */
function pretendDesktop() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

// Every implementation is restored here, not just the calls. A test that makes
// one call reject would otherwise leave it rejecting for every test after it,
// which is the order-dependent failure this suite is least able to explain.
beforeEach(() => {
  vi.clearAllMocks();
  win.resized.length = 0;
  win.isMaximized.mockResolvedValue(false);
  win.onResized.mockImplementation(async (handler: () => void) => {
    win.resized.push(handler);
    return win.unlisten;
  });
  win.minimize.mockResolvedValue(undefined);
  win.toggleMaximize.mockResolvedValue(undefined);
  win.close.mockResolvedValue(undefined);
});

// vitest globals are off in this project (see vite.config.ts), so
// testing-library's auto-cleanup never engages on its own. The
// `__TAURI_INTERNALS__` delete matters just as much: desktop mode is global
// state, and leaving it set would silently put the *next* file's components
// into a shell that is not there.
afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("WindowControls", () => {
  it("renders minimize, maximize and close with accessible names", async () => {
    pretendDesktop();
    render(<WindowControls />);

    expect(screen.getByRole("button", { name: "Minimize" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Maximize" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();

    // The mount-time state read has to settle before the test ends, or its
    // `setState` lands after cleanup and React complains into the next test.
    await waitFor(() => expect(win.onResized).toHaveBeenCalled());
  });

  /**
   * The served LAN UI has no window at all — no titlebar was ever removed
   * there, and `close()` would have nothing to close. Absent, not disabled:
   * this asserts an empty container rather than three inert buttons.
   */
  it("renders nothing in a browser", () => {
    const { container } = render(<WindowControls />);

    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(win.isMaximized).not.toHaveBeenCalled();
    expect(win.onResized).not.toHaveBeenCalled();
  });

  /**
   * Tauri's drag-region script walks the event path and refuses to start a
   * window drag once it meets a `BUTTON`. That rule — not anything written
   * here — is what stops a press on these controls from dragging the window
   * instead, so the tag is a contract rather than an implementation detail.
   */
  it("draws real buttons, which is what stops the drag region swallowing them", async () => {
    pretendDesktop();
    render(<WindowControls />);

    for (const name of ["Minimize", "Maximize", "Close"]) {
      expect(screen.getByRole("button", { name }).tagName).toBe("BUTTON");
    }
    await waitFor(() => expect(win.onResized).toHaveBeenCalled());
  });

  it("asks the window to minimize, toggle maximize, and close", async () => {
    pretendDesktop();
    render(<WindowControls />);
    await waitFor(() => expect(win.onResized).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Minimize" }));
    await waitFor(() => expect(win.minimize).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Maximize" }));
    await waitFor(() => expect(win.toggleMaximize).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(win.close).toHaveBeenCalledTimes(1));
  });

  it("says Restore when the window is already maximized on mount", async () => {
    pretendDesktop();
    win.isMaximized.mockResolvedValue(true);
    render(<WindowControls />);

    expect(await screen.findByRole("button", { name: "Restore" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Maximize" })).not.toBeInTheDocument();
  });

  /**
   * The staleness that matters. Double-clicking the titlebar, dragging it to
   * the top edge and Win+Up all maximise the window without touching this
   * component, and each of them arrives here as nothing but a resize.
   */
  it("follows a maximize it did not perform, and a restore after it", async () => {
    pretendDesktop();
    render(<WindowControls />);
    await waitFor(() => expect(win.resized).toHaveLength(1));

    win.isMaximized.mockResolvedValue(true);
    win.resized[0]();
    expect(await screen.findByRole("button", { name: "Restore" })).toBeInTheDocument();

    win.isMaximized.mockResolvedValue(false);
    win.resized[0]();
    expect(await screen.findByRole("button", { name: "Maximize" })).toBeInTheDocument();
  });

  /**
   * Dragging a window edge emits a resize per frame, and each one starts its
   * own read. They can come back in any order, and the icon must end up
   * showing the newest answer rather than the last one to arrive.
   */
  it("ignores an out-of-order read rather than stranding on a stale answer", async () => {
    pretendDesktop();
    const pending: Array<(value: boolean) => void> = [];
    win.isMaximized.mockImplementation(
      () => new Promise<boolean>((resolve) => pending.push(resolve)),
    );

    render(<WindowControls />);
    // The mount read, which only starts once the subscription is up.
    await waitFor(() => expect(pending).toHaveLength(1));
    await act(async () => pending[0](false));
    expect(screen.getByRole("button", { name: "Maximize" })).toBeInTheDocument();

    // Two resizes, back to back, each with a read of its own in flight.
    win.resized[0]();
    win.resized[0]();
    expect(pending).toHaveLength(3);

    // The newer read lands first and says the window is now maximized.
    await act(async () => pending[2](true));
    expect(screen.getByRole("button", { name: "Restore" })).toBeInTheDocument();

    // The older read lands late with the state as it was before. Applying it
    // would leave "Maximize" on a maximized window until the next resize.
    await act(async () => pending[1](false));
    expect(screen.getByRole("button", { name: "Restore" })).toBeInTheDocument();
  });

  it("stops listening when it goes away", async () => {
    pretendDesktop();
    const { unmount } = render(<WindowControls />);
    await waitFor(() => expect(win.onResized).toHaveBeenCalled());

    unmount();

    expect(win.unlisten).toHaveBeenCalledTimes(1);
  });

  /**
   * A window that refuses to close must not take the app's console with it.
   * The buttons have nowhere to report a failure, so the one thing they owe is
   * to fail quietly rather than as an unhandled rejection.
   */
  it("survives a window call that rejects", async () => {
    pretendDesktop();
    win.close.mockRejectedValue(new Error("no"));
    render(<WindowControls />);
    await waitFor(() => expect(win.onResized).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    await waitFor(() => expect(win.close).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  /** A shell too old to answer leaves working buttons and a resting icon. */
  it("still renders when the window cannot be read", async () => {
    pretendDesktop();
    win.isMaximized.mockRejectedValue(new Error("no"));
    win.onResized.mockRejectedValue(new Error("no"));
    render(<WindowControls />);

    expect(screen.getByRole("button", { name: "Maximize" })).toBeInTheDocument();
    await waitFor(() => expect(win.onResized).toHaveBeenCalled());
  });
});

/**
 * A window must never be narrower than its own titlebar.
 *
 * With `decorations: false` there is no operating-system Close button behind
 * these three. Once the header overflows, the shell's `overflow-hidden` clips
 * it with no scrollbar and Close is the first thing to go — so a window
 * narrower than its titlebar is a window that cannot be closed with the mouse.
 */
describe("the window's minimum width", () => {
  /**
   * Measured in headless Chromium against the header's real classes: while
   * recording, its min-content width is **488px**. Neither flex item shrinks
   * — both are `min-width: auto` — so below that the row overflows to the
   * right and is clipped. At the old `minWidth: 380` none of Close's 44px was
   * visible; it was whole again only at about 490.
   *
   * The floor is that measurement plus 12px of headroom, because 488 is one
   * point estimate: it moves with font metrics and display scaling, and the
   * consequence of being 2px short is a window you cannot close.
   *
   * Asserted as a number rather than measured here because jsdom implements no
   * layout — every `getBoundingClientRect` in this suite returns zeros, so
   * min-content width cannot be computed in this environment at all. Re-measure
   * in a real browser if the header ever gains another control.
   */
  const TITLEBAR_FLOOR_PX = 500;

  it("leaves room for the whole titlebar, window controls included", () => {
    expect(tauriConf.app.windows[0].minWidth).toBeGreaterThanOrEqual(TITLEBAR_FLOOR_PX);
  });

  /** The floor only means anything while the app draws its own titlebar. */
  it("is only load-bearing because the window is undecorated", () => {
    expect(tauriConf.app.windows[0].decorations).toBe(false);
  });
});
