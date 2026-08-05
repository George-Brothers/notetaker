/**
 * Tauri accelerator strings ("CommandOrControl+Alt+N") to and from the DOM.
 * `code` is the source of truth for the main key so a layout that moves
 * letters (AZERTY) still records the physical key the OS will match.
 */
import { modKey } from "../components/ui";

const MODIFIER_KEYS = new Set(["Control", "Meta", "Alt", "Shift", "AltGraph", "OS"]);

/**
 * Whether an accelerator is safe to hand to the OS as a *global* shortcut.
 *
 * Ctrl/Cmd or Alt has to be in it, and Shift on its own does not count.
 * `Shift+N` is simply how a keyboard produces a capital N, so registering it
 * system-wide would fire — and on Windows swallow — every capital N typed in
 * every application on the computer. Exported because two layers need the same
 * answer: the capture field, which declines the chord as you press it, and the
 * hook that registers whatever `settings.json` already holds, which may predate
 * this rule or have been edited by hand.
 */
export function isSafeAccelerator(accelerator: string): boolean {
  const parts = accelerator.split("+");
  return parts.includes("CommandOrControl") || parts.includes("Alt");
}

export function acceleratorFromEvent(e: {
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("CommandOrControl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  const main = mainKeyName(e.code, e.key);
  if (!main) return null;
  parts.push(main);
  const accelerator = parts.join("+");
  // Ctrl/Cmd or Alt is required, and Shift alone is not enough — see
  // `isSafeAccelerator`. `null` is this function's "not a hotkey yet, keep
  // listening" signal, so the field simply declines the chord and says nothing.
  if (!isSafeAccelerator(accelerator)) return null;
  return accelerator;
}

function mainKeyName(code: string, key: string): string | null {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (code === "Space") return "Space";
  if (/^F\d{1,2}$/.test(code)) return code;
  const named: Record<string, string> = {
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Escape: "Escape", Enter: "Enter", Backspace: "Backspace", Delete: "Delete",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Tab: "Tab",
  };
  if (named[code]) return named[code];
  // Punctuation rows: fall back to the produced key when it is one printable char.
  if (key.length === 1 && key !== " ") return key.toUpperCase();
  return null;
}

export function formatAcceleratorParts(accel: string): string[] {
  return accel.split("+").map((p) => (p === "CommandOrControl" ? modKey() : p));
}
