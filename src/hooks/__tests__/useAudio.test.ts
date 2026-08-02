import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { clampSeek, readRate, RATES, SKIP_SECONDS, useAudio, RATE_KEY } from "../useAudio";

vi.mock("../../lib/transport", () => ({
  audioSrc: vi.fn().mockResolvedValue("blob:audio"),
}));

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

describe("useAudio hook", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it("seek sets currentTime but does not call play", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.seek(30);
    });

    expect(fakeElement.currentTime).toBe(30);
    expect(mockPlay).not.toHaveBeenCalled();
    expect(result.current.currentTime).toBe(30);
  });

  it("seekAndPlay sets currentTime and calls play", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.seekAndPlay(30);
    });

    expect(fakeElement.currentTime).toBe(30);
    expect(mockPlay).toHaveBeenCalledTimes(1);
  });

  it("seek clamps to the start when given a negative position", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.seek(-5);
    });

    expect(fakeElement.currentTime).toBe(0);
    expect(result.current.currentTime).toBe(0);
  });

  it("seek clamps to the end when seeking past duration", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.seek(150);
    });

    expect(fakeElement.currentTime).toBe(100);
    expect(result.current.currentTime).toBe(100);
  });

  it("skip moves relative to current position and does not call play", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 30,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.skip(10);
    });

    expect(fakeElement.currentTime).toBe(40);
    expect(mockPlay).not.toHaveBeenCalled();
  });

  it("skip with negative delta moves backward", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 30,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.skip(-10);
    });

    expect(fakeElement.currentTime).toBe(20);
    expect(mockPlay).not.toHaveBeenCalled();
  });

  it("toggle calls play when element is paused", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.toggle();
    });

    expect(mockPlay).toHaveBeenCalledTimes(1);
    expect(mockPause).not.toHaveBeenCalled();
  });

  it("toggle calls pause when element is playing", async () => {
    const mockPlay = vi.fn().mockResolvedValue(undefined);
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: false,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    await act(async () => {
      result.current.toggle();
    });

    expect(mockPlay).not.toHaveBeenCalled();
    expect(mockPause).toHaveBeenCalledTimes(1);
  });

  it("setRate persists to localStorage and is restored on new hook", async () => {
    const { result: hook1 } = renderHook(() => useAudio("rec1", "primary", 100));

    await act(async () => {
      hook1.current.setRate(1.5);
    });

    expect(localStorage.getItem(RATE_KEY)).toBe("1.5");

    // Fresh hook should restore the saved rate
    const { result: hook2 } = renderHook(() => useAudio("rec1", "primary", 100));

    expect(hook2.current.rate).toBe(1.5);
  });

  it("setRate falls back to 1× when localStorage throws (private mode)", async () => {
    const getItemSpy = vi.spyOn(localStorage, "getItem").mockImplementation(() => {
      throw new Error("Private mode");
    });
    const setItemSpy = vi.spyOn(localStorage, "setItem").mockImplementation(() => {
      throw new Error("Private mode");
    });

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    // Initial rate should fall back to 1 when getItem throws
    expect(result.current.rate).toBe(1);

    // setRate should not throw even when setItem fails
    await act(async () => {
      result.current.setRate(1.5);
    });

    // Should succeed without throwing
    expect(result.current.rate).toBe(1.5);

    getItemSpy.mockRestore();
    setItemSpy.mockRestore();
  });

  it("seekAndPlay handles autoplay rejection silently", async () => {
    const mockPlay = vi
      .fn()
      .mockRejectedValue(new Error("Autoplay denied"));
    const mockPause = vi.fn();
    const fakeElement = {
      play: mockPlay,
      pause: mockPause,
      currentTime: 0,
      paused: true,
    };

    const { result } = renderHook(() => useAudio("rec1", "primary", 100));

    act(() => {
      result.current.ref.current = fakeElement as any;
    });

    // Should not throw even though play rejects
    await act(async () => {
      result.current.seekAndPlay(30);
    });

    expect(fakeElement.currentTime).toBe(30);
    expect(mockPlay).toHaveBeenCalledTimes(1);
  });
});
