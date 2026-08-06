/**
 * The contract between the React UI and the Rust core.
 *
 * Every field name here matches a serde `rename_all = "camelCase"` struct on
 * the Rust side; every function matches a `#[tauri::command]`. Changing a name
 * here without changing it there breaks silently at runtime, so this file is
 * the single place both sides agree on.
 *
 * Desktop-shell-only commands (ones with no meaning for the LAN/web build,
 * like enumerating input devices) are deliberately NOT in the `api` object
 * below — putting them here would let a served browser tab call them. Those
 * live in `./desktop.ts` instead, each guarded by its own `isDesktop()` check.
 */
// Tauri IPC on the desktop, HTTP when this UI is served to a browser. Same
// command names and the same camelCase arguments either way, so nothing below
// changes. See transport.ts.
import { invoke } from "./transport";

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
   * A better title than the auto-generated timestamp, awaiting the same
   * one-click confirmation. null when there is nothing better to offer.
   */
  suggestedTitle: string | null;
  /** Whether the user typed notes on this recording. Just the flag. */
  hasNotes: boolean;
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
  /** True only in the Archive view; archived recordings are not searched. */
  archived?: boolean;
}

/** One checkbox line parsed out of `summaryMd`. */
export interface ActionItem {
  /**
   * Position among the checkbox lines, 0-based. What `setActionDone` takes.
   * Not stable across an edit that adds or removes an item, which is why
   * `setActionDone` returns the whole re-parsed list.
   */
  index: number;
  /** The line with its `- [ ] ` marker stripped, owner prefix included. */
  text: string;
  /** The `Name:` prefix if the line has one that looks like a person. */
  owner: string | null;
  done: boolean;
  /** 0-based line in `summaryMd`, for scrolling to the item. */
  line: number;
}

/** One timed stretch of the transcript, for the player. */
export interface Segment {
  startS: number;
  /** The next segment's start, or the recording's duration for the last. */
  endS: number;
  /** Empty for a line with no speaker tag. */
  speaker: string;
  text: string;
  /** 0-based line in `transcriptMd`. */
  line: number;
}

/** A note shape the summary can be written to. */
export interface Template {
  id: string;
  name: string;
  blurb: string;
}

export interface RecordingDetail extends RecordingRow {
  transcriptMd: string;
  summaryMd: string;
  /** Diarizer key ("spk1") to display name ("Jamie"). */
  speakers: Record<string, string>;
  /**
   * The user's own notes, typed during the meeting. Never rewritten by the
   * app — the AI's output goes to `summaryMd` instead, which is what lets the
   * UI show your words at full contrast and the model's in grey.
   */
  notesMd: string;
  /** Which template shapes the summary. null means the default. */
  template: string | null;
  /**
   * The checklist, parsed from `summaryMd` rather than stored separately, so
   * it can never disagree with the markdown the user can edit by hand.
   */
  actions: ActionItem[];
  /**
   * The transcript as timed segments. Empty when the recording is
   * unprocessed, or when its transcript has been rewritten as prose — render
   * `transcriptMd` directly in that case.
   */
  segments: Segment[];
  /**
   * Which audio tracks exist on disk ("mic", "system"). Offer playback only
   * for these: an in-person recording has no system track.
   */
  audioTracks: string[];
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
  /** Optional summary-model overrides keyed by note-folder name. */
  taskModels: Record<string, string>;
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
  /**
   * The languages spoken in this user's recordings, as ISO-639-1 codes.
   * First run asks; this is the answer, and it decides which speech models
   * get downloaded. Defaults to `["en"]`.
   */
  languages: string[];
  /**
   * Which speech model transcribes. `auto` detects each segment's language
   * and sends it to the better model; the other two force one.
   */
  speechEngine: SpeechEngine;
  /** Which input device records. null means the system default. */
  inputDevice: string | null;
  /** Global start/stop-recording accelerator, Tauri notation. */
  hotkeyToggleRecord: string;
  /** Global show/hide-window accelerator, Tauri notation. */
  hotkeyShowHide: string;
  /** Global star-this-moment accelerator, Tauri notation. */
  hotkeyHighlight: string;
  /** Closing the window hides to the tray instead of quitting. */
  closeToTray: boolean;
  /** When the floating meeting overlay appears. Desktop-shell-only. */
  overlay: OverlayMode;
  /** Preferred microphone order; an empty list means the system default. */
  audioDevicePriority: string[];
  /** High-level model performance preference. */
  performanceMode: PerformanceMode;
  /** When speech models may be unloaded after their last use. */
  modelIdleUnload: ModelIdleUnload;
  /** Ollama model for the dictation cleanup pass. */
  cleanupModel: string;
  /** Whether the dictation cleanup pass is enabled. */
  dictationCleanupEnabled: boolean;
  /** Words/names supplied to the dictation recognizer. */
  dictationDictionary: string[];
  /** Spoken form -> corrected form. */
  dictationReplacements: Record<string, string>;
  /** Hold the shortcut or press it twice. */
  dictationMode: DictationMode;
  /** Insert at the cursor or copy without inserting. */
  dictationPasteBehavior: PasteBehavior;
  /** Reserved system-wide dictation accelerator. */
  dictationHotkey: string;
  /** Keep a local audio copy of dictation history. */
  dictationKeepAudio: boolean;
  /** Desktop overlay placement. */
  overlayPosition: OverlayPosition;
  /** Desktop overlay visual treatment. */
  overlayStyle: OverlayStyle;
  /** Ask the OS to exclude the overlay from capture where supported. */
  overlayHideFromShare: boolean;
}

export type SpeechEngine = "auto" | "whisper" | "senseVoice";

export type PerformanceMode = "auto" | "bestQuality" | "cpuOptimized";
export type ModelIdleUnload = "never" | "afterBatch" | "15s" | "2m" | "5m" | "15m" | "1h";
export type DictationMode = "pushToTalk" | "toggle";
export type PasteBehavior = "paste" | "copyOnly";
export type OverlayPosition = "topRight" | "topCenter" | "bottomCenter";
export type OverlayStyle = "glass" | "solid";

/**
 * When the floating overlay (the always-on-top recording pill) shows:
 * never, with each recording, or from the moment a meeting is detected —
 * where it doubles as the "record this?" prompt.
 */
export type OverlayMode = "off" | "recording" | "meeting";

/** One model the app needs and does not have. */
export interface MissingModel {
  name: string;
  /** What it is, in words meant for a person. */
  label: string;
  bytes: number;
}

/** A matching-size model file found outside Notetaker's own model folder. */
export interface FoundModel {
  name: string;
  label: string;
}

/**
 * What the app can and cannot do right now.
 *
 * The app never refuses to record over this — recording works with nothing
 * downloaded at all. This exists so the interface can say what will not happen
 * rather than accepting work it will silently never do.
 */
export interface SetupStatus {
  /** False means recordings are captured and kept, and nothing transcribes them. */
  transcribing: boolean;
  missing: MissingModel[];
  /** What `missing` would cost to download. */
  downloadBytes: number;
  /** Recordings already waiting that downloading would unblock. */
  waiting: number;
  /** The hardware tier the model choice was made for. */
  tier: string;
}

/**
 * The languages the app can be told about, and what choosing one costs.
 *
 * `senseVoice: true` means picking it downloads a second 239 MB model that is
 * markedly better at that language than Whisper. Everything else is handled by
 * Whisper, which is downloaded either way — so those choices are free.
 */
export const LANGUAGE_CHOICES: ReadonlyArray<{
  code: string;
  label: string;
  senseVoice: boolean;
}> = [
  { code: "en", label: "English", senseVoice: false },
  { code: "zh", label: "Chinese (Mandarin)", senseVoice: true },
  { code: "yue", label: "Cantonese", senseVoice: true },
  { code: "ja", label: "Japanese", senseVoice: true },
  { code: "ko", label: "Korean", senseVoice: true },
  { code: "other", label: "Another language", senseVoice: false },
];

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

/** Lightweight, fast-moving readings for the record-bar meters. */
export interface CaptureLevels {
  micLevel: number;
  systemLevel: number;
}

/** One chunked-batch transcript update for the expanded overlay. */
export interface LiveTranscriptEvent {
  speaker: "me" | "them";
  text: string;
  isPartial: boolean;
  isFinal: boolean;
}

/** One incremental local-Ollama Ask update for the expanded overlay. */
export interface LiveAskEvent {
  token: string;
  done: boolean;
  error: string | null;
}

/** Status of the independent microphone-only dictation slot. */
export type DictationState = "idle" | "recording" | "transcribing" | "pasting" | "error";

export interface DictationStatus {
  state: DictationState;
  elapsedS: number;
  level: number;
  text: string;
  message: string | null;
}

export interface PasteResult {
  inserted: boolean;
  clipboardRestored: boolean;
  message: string;
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
  listArchivedRecordings: () => invoke<RecordingRow[]>("list_archived_recordings"),
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
  /** Moves a recording to Archive without deleting any of its files. */
  archiveRecording: (id: string) => invoke<void>("archive_recording", { id }),
  /** Returns an archived recording to Unsorted. */
  restoreRecording: (id: string) => invoke<void>("restore_recording", { id }),
  /** Permanent; the interface asks for confirmation before calling this. */
  deleteRecording: (id: string) => invoke<void>("delete_recording", { id }),
  renameSpeaker: (id: string, key: string, name: string) =>
    invoke<void>("rename_speaker", { id, key, name }),

  // --- The notepad (Plan C) ---
  /**
   * Saves what the user typed. Safe to call while the recording is still
   * running — writing notes touches nothing the capture threads own, which is
   * why this is allowed where renaming and filing are refused.
   */
  saveNotes: (id: string, notesMd: string) =>
    invoke<void>("save_notes", { id, notesMd }),
  /** Appends a jot from the expanded overlay; it never rewrites notes.md. */
  appendNote: (id: string, jot: string) => invoke<void>("append_note", { id, jot }),
  /**
   * Stars the current moment of the live recording. No arguments: the
   * runtime knows the live recording and its clock better than any frontend
   * snapshot. Returns the appended line ("- ⭐ 0:12:34").
   */
  addHighlight: () => invoke<string>("add_highlight"),
  /** Every note shape available in the picker. */
  listTemplates: () => invoke<Template[]>("list_templates"),
  /**
   * Sets which template shapes this recording's summary. Takes effect on the
   * next processing run, not retroactively — say so in the UI, or picking a
   * template looks like it did nothing.
   */
  setTemplate: (id: string, template: string) =>
    invoke<void>("set_template", { id, template }),
  /**
   * Ticks or unticks one action item, which rewrites that line of the
   * summary. Returns the whole re-parsed list, because indices shift if the
   * summary was edited in the meantime.
   */
  setActionDone: (id: string, index: number, done: boolean) =>
    invoke<ActionItem[]>("set_action_done", { id, index, done }),
  /**
   * One question about one recording, answered locally from its own notes,
   * summary and transcript. Stateless: no conversation history, the whole
   * recording is re-sent each time.
   */
  askRecording: (id: string, question: string) =>
    invoke<string>("ask_recording", { id, question }),
  /** Starts a local-only streamed answer over the rolling live transcript. */
  startLiveAsk: (question: string, context: string) =>
    invoke<string>("start_live_ask", { question, context }),
  /** Drains incremental tokens for one live Ask request. */
  pollLiveAsk: (id: string) => invoke<LiveAskEvent[]>("poll_live_ask", { id }),
  /**
   * The absolute path to an audio track. Not playable as-is in a browser —
   * pass it through `audioSrc` in transport.ts, which turns it into whatever
   * the current transport can actually load.
   */
  audioPath: (id: string, track: string) =>
    invoke<string>("audio_path", { id, track }),
  /** The current log file, which the desktop shell can reveal to the user. */
  logPath: () => invoke<string>("log_path"),
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
  captureLevels: () => invoke<CaptureLevels>("capture_levels"),
  /** Drains transcript updates produced by the read-only capture tee. */
  liveTranscript: () => invoke<LiveTranscriptEvent[]>("live_transcript"),
  startDictation: () => invoke<DictationStatus>("start_dictation"),
  stopDictation: () => invoke<DictationStatus>("stop_dictation"),
  cancelDictation: () => invoke<DictationStatus>("cancel_dictation"),
  dictationStatus: () => invoke<DictationStatus>("dictation_status"),
  copyLastTranscript: () => invoke<PasteResult>("copy_last_transcript"),

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
  /** Hash-verify and copy the existing speech model candidates the user accepted. */
  adoptModels: () => invoke<void>("adopt_models"),
  /** Bounded first-run scan for speech models already on this computer. */
  findExistingModels: () => invoke<FoundModel[]>("find_existing_models"),
  /** The hardware tier detected for this machine ("small" | "medium" | "large"). */
  detectedTier: () => invoke<string>("detected_tier"),
  /**
   * What the app can and cannot do right now, read from disk rather than from
   * whatever this session remembers downloading. Safe to poll; it does no
   * network work.
   */
  setupStatus: () => invoke<SetupStatus>("setup_status"),
};
