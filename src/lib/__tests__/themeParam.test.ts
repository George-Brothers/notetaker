import { describe, expect, it, vi } from "vitest";
import { applyThemeParam } from "../themeParam";

describe("applyThemeParam", () => {
  it("stores a valid ?theme= value under the useTheme key", () => {
    const setItem = vi.fn();
    applyThemeParam("?theme=dark", { setItem });
    expect(setItem).toHaveBeenCalledWith("notetaker.theme", "dark");
  });

  it("ignores absent and invalid values", () => {
    const setItem = vi.fn();
    applyThemeParam("", { setItem });
    applyThemeParam("?theme=neon", { setItem });
    expect(setItem).not.toHaveBeenCalled();
  });

  it("swallows storage errors", () => {
    const setItem = vi.fn(() => {
      throw new Error("private mode");
    });
    expect(() => applyThemeParam("?theme=light", { setItem })).not.toThrow();
  });
});
