/**
 * The contract between the React UI and the Rust core.
 *
 * Every field name here matches a serde `rename_all = "camelCase"` struct on
 * the Rust side; every function matches a `#[tauri::command]`. Changing a name
 * here without changing it there breaks silently at runtime, so this file is
 * the single place both sides agree on.
 */
import { invoke } from "@tauri-apps/api/core";

export type Status = "recorded" | "queued" | "processing" | "ready" | "failed";
export type Mode = "meeting" | "in_person";

export interface RecordingRow {
  id: string;
  title: string;
  /** null means the recording is still in Unsorted. */
  task: string | null;
  /** RFC3339. */
  created: string;
  durationS: number;
  mode: Mode;
  status: Status;
  /** What the AI thinks this belongs to, awaiting one-click confirmation. */
  suggestedTask: string | null;
  /**
   * Why processing failed, in language a non-technical user can act on.
   * Present on list rows too, so a failed row can explain itself without
   * making the user open it.
   */
  error: string | null;
  /**
   * Why *capture* ended early or lost a track, if it did. Separate from
   * `error` because it outlives every processing attempt — a finished, fully
   * transcribed recording still needs to be able to say "this is short
   * because the disk filled up."
   */
  captureNote: string | null;
}

export interface RecordingDetail extends RecordingRow {
  transcriptMd: string;
  summaryMd: string;
  /** Diarizer key ("spk1") to display name ("Jamie"). */
  speakers: Record<string, string>;
}

export interface SearchHit {
  id: string;
  title: string;
  task: string | null;
  snippet: string;
}

/** What the app does when a known meeting app appears. */
export type AutoRecordPolicy = "ask" | "always" | "never";

export interface Settings {
  storageRoot: string;
  llmBaseUrl: string;
  llmModel: string;
  /** null means "use the tier detected from this machine's hardware". */
  tierOverride: string | null;
  processWhenIdle: boolean;
  /** Per-app policy, keyed by app id ("zoom"). Missing key means "ask". */
  autoRecord: Record<string, AutoRecordPolicy>;
  /** Inactivity required before background processing starts. */
  minIdleSecs: number;
  /** Only process on wall power. */
  requireAc: boolean;
  /** Keep the intermediate WAV after the lossless FLAC finalize. */
  keepWav: boolean;
}

/**
 * `finishing` is the stretch after the last sample and before the recording is
 * queued: the tracks are still being re-encoded and indexed. It is not idle —
 * re-arming the record bar there shows the user a library with nothing new in
 * it — and it is not recording either.
 */
export type CaptureState = "idle" | "recording" | "paused" | "finishing";

/** Live snapshot for the record bar. Polled while recording. */
export interface CaptureStatus {
  state: CaptureState;
  /**
   * What kind of recording is running; null when nothing is. Carried here
   * rather than remembered by whichever component pressed Record, so a menu
   * bar or a window reopened mid-recording can tell a meeting from a lecture
   * without having started it.
   */
  mode: Mode | null;
  recordingId: string | null;
  /** Seconds of audio captured; paused time is not counted. */
  elapsedS: number;
  /** Peak level since the last poll, 0..1. */
  micLevel: number;
  /** Always 0 for in-person recordings, which have no system track. */
  systemLevel: number;
  diskFreeMb: number;
}

export type MeetingEventKind = "started" | "ended";

/** A meeting app appearing or disappearing, after debounce. */
export interface MeetingEvent {
  appId: string;
  appName: string;
  kind: MeetingEventKind;
  /** True when policy is "always" — record without prompting. */
  autoStart: boolean;
}

/** Whether the local LLM is ready, and what's missing if not. */
export interface OllamaStatus {
  installed: boolean;
  running: boolean;
  models: string[];
  /** Whether `Settings.llmModel` is among `models`. */
  modelReady: boolean;
  /** Present when not installed: what the user should do about it. */
  installHint: string | null;
}

/**
 * Which checklist item a download belongs to. The first-run screen tracks
 * "download the speech models" and "install the summary AI" separately, but
 * both report through one progress list.
 */
export type PullKind = "ollama" | "speech";

/** Progress of a model download — Ollama pulls and whisper/sherpa fetches. */
export interface PullProgress {
  kind: PullKind;
  /** What is being fetched, for the label. */
  name: string;
  /** 0..100. */
  percent: number;
  /** Terminal states carry a message; otherwise null. */
  error: string | null;
  done: boolean;
}

export const api = {
  listTasks: () => invoke<string[]>("list_tasks"),
  createTask: (name: string) => invoke<void>("create_task", { name }),
  listRecordings: () => invoke<RecordingRow[]>("list_recordings"),
  getRecording: (id: string) => invoke<RecordingDetail>("get_recording", { id }),
  search: (query: string) => invoke<SearchHit[]>("search", { query }),
  processNow: (id: string) => invoke<void>("process_now", { id }),
  /** Persists a user's edits to the AI-written summary. */
  updateSummary: (id: string, summaryMd: string) =>
    invoke<void>("update_summary", { id, summaryMd }),
  assignTask: (id: string, task: string) => invoke<void>("assign_task", { id, task }),
  /**
   * Renames a recording. Recordings start with an auto-generated title
   * ("Meeting — Jul 27, 2:30 PM") so that hitting record never blocks on
   * typing; this is how one becomes "Accounting 302 — midterm review" later.
   * The title is part of the on-disk directory name, so this moves the
   * directory too.
   */
  renameRecording: (id: string, title: string) =>
    invoke<void>("rename_recording", { id, title }),
  renameSpeaker: (id: string, key: string, name: string) =>
    invoke<void>("rename_speaker", { id, key, name }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),

  // --- Capture (Plan B) ---
  startCapture: (mode: Mode, title: string) =>
    invoke<CaptureStatus>("start_capture", { mode, title }),
  pauseCapture: () => invoke<CaptureStatus>("pause_capture"),
  resumeCapture: () => invoke<CaptureStatus>("resume_capture"),
  /** Stops, finalizes to FLAC, and queues the recording. Returns its id. */
  stopCapture: () => invoke<string>("stop_capture"),
  captureStatus: () => invoke<CaptureStatus>("capture_status"),

  // --- Meeting watcher ---
  /** Drains any debounced meeting events since the last poll. */
  pollMeetings: () => invoke<MeetingEvent[]>("poll_meetings"),
  setAutoRecord: (appId: string, policy: AutoRecordPolicy) =>
    invoke<void>("set_auto_record", { appId, policy }),

  // --- Local models ---
  ollamaStatus: () => invoke<OllamaStatus>("ollama_status"),
  /** Starts a pull; progress arrives via `pullProgress`. */
  pullModel: (model: string) => invoke<void>("pull_model", { model }),
  pullProgress: () => invoke<PullProgress[]>("pull_progress"),
  /**
   * Starts downloading the speech models this machine's tier needs. Models
   * already present report complete without re-downloading, so pressing this
   * twice is harmless. Progress arrives via `pullProgress` with kind "speech".
   */
  downloadModels: () => invoke<void>("download_models"),
  /** The hardware tier detected for this machine ("small" | "medium" | "large"). */
  detectedTier: () => invoke<string>("detected_tier"),
};
