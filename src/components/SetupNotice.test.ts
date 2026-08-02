/**
 * The wording is the whole feature, so the wording is what gets tested.
 *
 * A user recorded three meetings on a machine with no speech models, pressed
 * "Process now", and was told nothing at all — because nothing had failed.
 * These assertions pin the two things that must never come back: silence when
 * the app cannot do the job, and noise when it can.
 */

import { describe, expect, it } from "vitest";
import { formatBytes, setupMessage } from "./SetupNotice";
import type { SetupStatus } from "../lib/ipc";

const base: SetupStatus = {
  transcribing: false,
  missing: [{ name: "whisper-small-q5_1", label: "Speech model (fast)", bytes: 190085487 }],
  downloadBytes: 223574481,
  waiting: 3,
  tier: "CpuSmall",
};

describe("setupMessage", () => {
  it("says nothing at all when transcription is running", () => {
    expect(setupMessage({ ...base, transcribing: true, missing: [], downloadBytes: 0 })).toBeNull();
  });

  it("says nothing before the status has loaded", () => {
    expect(setupMessage(null)).toBeNull();
  });

  it("names the real download size rather than hand-waving", () => {
    const message = setupMessage(base);
    expect(message).toContain("224 MB");
  });

  it("says recording still works, because it does", () => {
    expect(setupMessage(base)).toMatch(/recording works/i);
  });

  it("counts the recordings that are stuck, in the plural", () => {
    expect(setupMessage(base)).toContain("3 recordings are waiting");
  });

  it("counts one stuck recording in the singular", () => {
    expect(setupMessage({ ...base, waiting: 1 })).toContain("One recording is waiting");
  });

  it("mentions no waiting recordings when there are none", () => {
    expect(setupMessage({ ...base, waiting: 0 })).not.toMatch(/waiting/i);
  });

  it("does not send you to download things you already have", () => {
    const message = setupMessage({ ...base, missing: [], downloadBytes: 0 });
    // It may well say the models *are* downloaded — that is the point. What it
    // must never do is claim they are missing and send you to fetch them
    // again, which is the one action that cannot help here.
    expect(message).toMatch(/restarting/i);
    expect(message).not.toMatch(/aren't downloaded|not downloaded/i);
  });
});

describe("formatBytes", () => {
  it("uses GB past a billion, so 1.6 GB does not read as 1625 MB", () => {
    expect(formatBytes(1624555275)).toBe("1.6 GB");
  });

  it("rounds MB to something a person would say", () => {
    expect(formatBytes(190085487)).toBe("190 MB");
  });

  it("never reports a real file as 0 KB", () => {
    expect(formatBytes(315894)).toBe("316 KB");
    expect(formatBytes(12)).toBe("1 KB");
  });
});
