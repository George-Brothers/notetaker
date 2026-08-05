/**
 * The window's own minimise, maximise and close.
 *
 * `decorations: false` takes the operating system's titlebar away, and the
 * three buttons every window is expected to have go with it. These are the
 * replacements. They are square rather than rounded and sit flush in the
 * top-right corner deliberately: that is where a mouse gets thrown without
 * looking, and a rounded button inset from the edge means the last few pixels
 * of the window do nothing.
 *
 * Nothing renders in a browser. The served LAN UI has no window to minimise,
 * and a control that looks live but cannot work is worse than no control — so
 * this returns `null` there rather than rendering three disabled buttons.
 *
 * These are plain `<button>` elements rather than the shared `IconButton` for
 * two reasons. The shape is wrong — `IconButton` is a rounded 32px square with
 * a tooltip, and window controls are flush rectangles that nobody needs a
 * tooltip to understand. And the tag matters: Tauri's drag-region script treats
 * `BUTTON` as clickable and refuses to start a window drag from one, which is
 * exactly what keeps these from dragging the window instead of pressing.
 */

import { useEffect, useState } from "react";
import { Copy, Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isDesktop } from "../lib/transport";
import { cn } from "../lib/cn";

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  /**
   * Keeps the middle icon honest.
   *
   * The window can be maximised and restored without this component's button
   * ever being pressed — double-clicking the titlebar, dragging it to the top
   * edge, Win+Up, Win+Down, the taskbar's own restore. An icon that still says
   * "maximise" on a maximised window is a small lie the user has to click to
   * disprove, so the state follows the *window* rather than the button:
   * `onResized` is the one event every one of those paths goes through, since
   * none of them can change the maximised state without changing the size.
   *
   * The failure mode if this ever misses one is cosmetic only. The button
   * calls `toggleMaximize`, which re-reads the real state on the Rust side, so
   * a stale icon still does the right thing when pressed.
   */
  useEffect(() => {
    if (!isDesktop()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    /**
     * Monotonic, so a reply that arrives out of order cannot win.
     *
     * `onResized` fires once per resize message — dozens of times during a
     * single drag of the window's edge — and each one starts its own
     * independent read. Nothing sequences those reads, so an earlier one that
     * happens to resolve last would strand the icon on an answer that was
     * already out of date when it arrived, until some later resize corrected
     * it. Each read carries its ticket number and only the newest may write.
     */
    let latest = 0;

    async function sync() {
      const seq = ++latest;
      try {
        const now = await getCurrentWindow().isMaximized();
        if (!cancelled && seq === latest) setMaximized(now);
      } catch {
        // Leave the icon where it is. Guessing would be worse than being one
        // state behind, and there is nowhere sensible to report this to.
      }
    }

    void (async () => {
      try {
        const stop = await getCurrentWindow().onResized(() => void sync());
        // Unmounted while the subscription was still in flight: drop it now
        // rather than leaving a listener behind with nothing to update.
        if (cancelled) stop();
        else unlisten = stop;
      } catch {
        // No subscription means the icon can go stale; the buttons still work.
      }
      // Read *after* subscribing, not before: a maximise landing in the gap
      // between the two would otherwise be missed by the listener that did not
      // exist yet and then overwritten by this read's stale answer.
      await sync();
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  if (!isDesktop()) return null;

  /**
   * `close()` is a request, not an exit: the Rust side intercepts
   * `CloseRequested`, prevents it, and hands the decision to the webview — so
   * this button hides to the tray or raises the stop-the-recording guard
   * exactly like the titlebar X used to.
   */
  async function run(action: "minimize" | "toggleMaximize" | "close") {
    if (!isDesktop()) return;
    try {
      const win = getCurrentWindow();
      if (action === "minimize") await win.minimize();
      else if (action === "toggleMaximize") await win.toggleMaximize();
      else await win.close();
    } catch {
      // Swallowed on purpose. There is no surface in a 32px-tall titlebar to
      // report "the window refused to minimise" on, and an unhandled rejection
      // in the console helps nobody.
    }
  }

  const base = cn(
    "inline-flex h-8 w-11 items-center justify-center text-fg-faint",
    "transition-colors hover:bg-hover hover:text-fg",
    "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent",
  );

  return (
    <div className="flex items-center" data-testid="window-controls">
      <button
        type="button"
        aria-label="Minimize"
        className={base}
        onClick={() => void run("minimize")}
      >
        <Minus size={14} />
      </button>
      <button
        type="button"
        aria-label={maximized ? "Restore" : "Maximize"}
        className={base}
        onClick={() => void run("toggleMaximize")}
      >
        {maximized ? <Copy size={12} /> : <Square size={12} />}
      </button>
      {/* The one control here you cannot undo announces itself on hover. */}
      <button
        type="button"
        aria-label="Close"
        className={cn(base, "hover:bg-recording hover:text-white")}
        onClick={() => void run("close")}
      >
        <X size={15} />
      </button>
    </div>
  );
}
