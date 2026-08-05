import { describe, expect, it } from "vitest";
import { acceleratorFromEvent, formatAcceleratorParts } from "../hotkeys";

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

describe("formatAcceleratorParts", () => {
  it("splits and renames the modifier for display", () => {
    expect(formatAcceleratorParts("CommandOrControl+Alt+N")).toEqual(["Ctrl", "Alt", "N"]);
  });
});
