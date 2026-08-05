import { describe, expect, it } from "vitest";
import { acceleratorFromEvent, formatAcceleratorParts, isSafeAccelerator } from "../hotkeys";

const ev = (over: Partial<Parameters<typeof acceleratorFromEvent>[0]>) => ({
  key: "n", code: "KeyN", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...over,
});

describe("acceleratorFromEvent", () => {
  it("builds CommandOrControl+Alt+N from ctrl+alt+n", () => {
    expect(acceleratorFromEvent(ev({ ctrlKey: true, altKey: true }))).toBe("CommandOrControl+Alt+N");
  });
  it("meta counts as CommandOrControl too", () => {
    expect(acceleratorFromEvent(ev({ metaKey: true, altKey: true }))).toBe("CommandOrControl+Alt+N");
  });
  it("returns null while only modifiers are down", () => {
    expect(acceleratorFromEvent(ev({ key: "Control", code: "ControlLeft", ctrlKey: true }))).toBeNull();
  });
  it("names Space and letters from code, not layout", () => {
    expect(acceleratorFromEvent(ev({ key: " ", code: "Space", ctrlKey: true, altKey: true }))).toBe("CommandOrControl+Alt+Space");
  });
  it("shift is carried", () => {
    expect(acceleratorFromEvent(ev({ ctrlKey: true, shiftKey: true }))).toBe("CommandOrControl+Shift+N");
  });
});

/**
 * The safety rule, from both ends.
 *
 * `Shift+N` is the one that matters: it has two parts, so a length check waves
 * it through, and it is also exactly what the keyboard sends for a capital N.
 * Registered OS-wide it would fire on every capital N typed anywhere on the
 * computer, which is a system-wide keyboard hijack, not a hotkey.
 */
describe("the modifier a global hotkey has to carry", () => {
  it("rejects Shift+N — that is just a capital N", () => {
    expect(acceleratorFromEvent(ev({ shiftKey: true }))).toBeNull();
    expect(isSafeAccelerator("Shift+N")).toBe(false);
  });
  it("still rejects a bare letter", () => {
    expect(acceleratorFromEvent(ev({}))).toBeNull();
    expect(isSafeAccelerator("N")).toBe(false);
  });
  it("accepts CommandOrControl+N", () => {
    expect(acceleratorFromEvent(ev({ ctrlKey: true }))).toBe("CommandOrControl+N");
    expect(isSafeAccelerator("CommandOrControl+N")).toBe(true);
  });
  it("accepts Alt+N", () => {
    expect(acceleratorFromEvent(ev({ altKey: true }))).toBe("Alt+N");
    expect(isSafeAccelerator("Alt+N")).toBe(true);
  });
  it("accepts CommandOrControl+Shift+N", () => {
    expect(acceleratorFromEvent(ev({ ctrlKey: true, shiftKey: true }))).toBe("CommandOrControl+Shift+N");
    expect(isSafeAccelerator("CommandOrControl+Shift+N")).toBe(true);
  });
});

describe("formatAcceleratorParts", () => {
  it("splits and renames the modifier for display", () => {
    expect(formatAcceleratorParts("CommandOrControl+Alt+N")).toEqual(["Ctrl", "Alt", "N"]);
  });
});
