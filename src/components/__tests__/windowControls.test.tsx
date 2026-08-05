import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WindowControls } from "../WindowControls";

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
