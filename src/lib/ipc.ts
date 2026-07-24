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
}

export interface RecordingDetail extends RecordingRow {
  transcriptMd: string;
  summaryMd: string;
  /** Diarizer key ("spk1") to display name ("Jamie"). */
  speakers: Record<string, string>;
  error: string | null;
}

export interface SearchHit {
  id: string;
  title: string;
  task: string | null;
  snippet: string;
}

export interface Settings {
  storageRoot: string;
  llmBaseUrl: string;
  llmModel: string;
  /** null means "use the tier detected from this machine's hardware". */
  tierOverride: string | null;
  processWhenIdle: boolean;
}

export const api = {
  listTasks: () => invoke<string[]>("list_tasks"),
  createTask: (name: string) => invoke<void>("create_task", { name }),
  listRecordings: () => invoke<RecordingRow[]>("list_recordings"),
  getRecording: (id: string) => invoke<RecordingDetail>("get_recording", { id }),
  search: (query: string) => invoke<SearchHit[]>("search", { query }),
  processNow: (id: string) => invoke<void>("process_now", { id }),
  assignTask: (id: string, task: string) => invoke<void>("assign_task", { id, task }),
  renameSpeaker: (id: string, key: string, name: string) =>
    invoke<void>("rename_speaker", { id, key, name }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
};
