import { describe, it, expect } from "vitest";
import { clampSeek, readRate, RATES, SKIP_SECONDS } from "../useAudio";

describe("clampSeek", () => {
  it("passes a position inside the recording straight through", () => {
    expect(clampSeek(42, 100)).toBe(42);
  });

  it("stops at the start rather than seeking to a negative time", () => {
    // What -10s does at 0:04.
    expect(clampSeek(4 - SKIP_SECONDS, 100)).toBe(0);
  });

  it("stops at the end rather than seeking past it", () => {
    // What +10s does at 0:57 of a one-minute recording.
    expect(clampSeek(57 + SKIP_SECONDS, 60)).toBe(60);
  });

  it("does not clamp against a duration the recording has not reported", () => {
    // durationS is 0 until a recording has been finalised. Clamping to 0
    // there would make the whole bar unseekable.
    expect(clampSeek(30, 0)).toBe(30);
  });

  it("treats a non-finite position as the start", () => {
    expect(clampSeek(Number.NaN, 100)).toBe(0);
  });
});

describe("RATES", () => {
  it("offers slower-than-normal as well as faster", () => {
    expect(RATES[0]).toBeLessThan(1);
    expect(RATES).toContain(1);
    expect(RATES[RATES.length - 1]).toBe(2);
  });
});

describe("readRate", () => {
  it("restores a speed that was saved last time", () => {
    expect(readRate("1.5")).toBe(1.5);
  });

  it("falls back to normal speed when nothing was saved", () => {
    expect(readRate(null)).toBe(1);
  });

  it("ignores a stored value that is not one of the offered speeds", () => {
    // Someone editing localStorage by hand should not get a 40× player.
    expect(readRate("40")).toBe(1);
    expect(readRate("banana")).toBe(1);
  });
});
