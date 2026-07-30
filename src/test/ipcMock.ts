/**
 * Shared defaults for the mocked `api`.
 *
 * Every test file mocks `../lib/ipc` with a factory that derives its keys from
 * the real `api` object:
 *
 * ```ts
 * vi.mock("../../lib/ipc", async (importOriginal) => {
 *   const actual = await importOriginal<typeof import("../../lib/ipc")>();
 *   return {
 *     ...actual,
 *     api: Object.fromEntries(Object.keys(actual.api).map((k) => [k, vi.fn()])),
 *   };
 * });
 * ```
 *
 * That shape is deliberate. The mocks used to be hand-written lists of command
 * names, one per test file, and adding a single command to `ipc.ts` turned all
 * 72 tests red with `api.listTemplates is not a function` — a failure that says
 * nothing about the code under test. Deriving the keys means a new command can
 * never do that again.
 *
 * What it cannot derive is *return shapes*: a command mocked to resolve
 * `undefined` still breaks a component that maps over its result. Hence
 * [`applyIpcDefaults`], which gives every command that returns a collection a
 * harmless empty one. Tests override whatever they actually care about.
 */

import { vi } from "vitest";
import { api } from "../lib/ipc";
import type { CaptureStatus } from "../lib/ipc";

export const IDLE_STATUS: CaptureStatus = {
  state: "idle",
  mode: null,
  recordingId: null,
  elapsedS: 0,
  micLevel: 0,
  systemLevel: 0,
  diskFreeMb: 20_000,
};

/**
 * Baseline resolved values, so a component that renders on mount finds an
 * empty library rather than `undefined`.
 *
 * Call from `beforeEach` after `vi.clearAllMocks()`.
 */
export function applyIpcDefaults(): void {
  // Everything resolves rather than rejecting, so an unrelated command never
  // produces an error banner that a test then trips over.
  for (const value of Object.values(api)) {
    if (vi.isMockFunction(value)) value.mockResolvedValue(undefined);
  }

  vi.mocked(api.listTasks).mockResolvedValue([]);
  vi.mocked(api.listRecordings).mockResolvedValue([]);
  vi.mocked(api.search).mockResolvedValue([]);
  vi.mocked(api.listTemplates).mockResolvedValue([
    { id: "default", name: "General notes", blurb: "A good default for any conversation." },
    { id: "lecture", name: "Lecture or class", blurb: "One person teaching." },
  ]);
  vi.mocked(api.setActionDone).mockResolvedValue([]);
  vi.mocked(api.pollMeetings).mockResolvedValue([]);
  vi.mocked(api.pullProgress).mockResolvedValue([]);
  vi.mocked(api.captureStatus).mockResolvedValue(IDLE_STATUS);
  vi.mocked(api.stopCapture).mockResolvedValue("rec-new");
  vi.mocked(api.detectedTier).mockResolvedValue("medium");
  vi.mocked(api.audioPath).mockResolvedValue("/tmp/audio-mic.flac");
  vi.mocked(api.askRecording).mockResolvedValue("");
  vi.mocked(api.ollamaStatus).mockResolvedValue({
    installed: true,
    running: true,
    models: [],
    modelReady: false,
    installHint: null,
  });
}
