/**
 * Tauri accelerator strings ("CommandOrControl+Alt+N") to and from the DOM.
 * `code` is the source of truth for the main key so a layout that moves
 * letters (AZERTY) still records the physical key the OS will match.
 */
import { modKey } from "../components/ui";

const MODIFIER_KEYS = new Set(["Control", "Meta", "Alt", "Shift", "AltGraph", "OS"]);

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
  // A bare letter with no modifier is not a global hotkey — it would swallow typing.
  if (parts.length === 1) return null;
  return parts.join("+");
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
