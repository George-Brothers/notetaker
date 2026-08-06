/**
 * The shell: a quiet top strip, the library rail, and the note.
 *
 * Two panes rather than the three this used to have. The old layout gave a
 * third of the window to a list of recordings and another third to a view
 * picker, which left the note — the only thing anyone is here to read — in a
 * column too narrow to read comfortably. The rail now does both jobs.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import { getCurrentWindow, primaryMonitor } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Moon, Settings as SettingsIcon, Sun } from "lucide-react";
import { useLibrary } from "./hooks/useLibrary";
import { useCapture } from "./hooks/useCapture";
import { useAutoUpdate } from "./hooks/useAutoUpdate";
import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useTheme } from "./hooks/useTheme";
import { Sidebar } from "./components/Sidebar";
import { NoteView } from "./components/NoteView";
import { RecordBar } from "./components/RecordBar";
import { MeetingPrompt } from "./components/MeetingPrompt";
import { Settings } from "./components/Settings";
import type { SettingsSection } from "./components/Settings";
import { FirstRun } from "./components/FirstRun";
import { SetupNotice } from "./components/SetupNotice";
import { CommandPalette } from "./components/CommandPalette";
import { WindowControls } from "./components/WindowControls";
import type { TrayModelState, TrayPanelSync } from "./components/TrayPanel";
import { Button, Dialog, IconButton, Notice, TooltipProvider } from "./components/ui";
import {
  api,
  type CaptureState,
  type CaptureStatus,
  type DictationStatus,
  type LiveTranscriptEvent,
  type Settings as SettingsData,
  type SetupStatus,
} from "./lib/ipc";
import { isDesktop } from "./lib/transport";
import { cn } from "./lib/cn";
import { duration } from "./lib/format";
import { isMacDesktop, listInputDevices, setAutostart, setTrayStatus } from "./lib/desktop";
import type { InputDevice } from "./lib/desktop";
import { formatAcceleratorParts } from "./lib/hotkeys";

const FIRST_RUN_DISMISSED_KEY = "notetaker.firstRunDismissed";
/** Set once the tray note has been read, so closing the window is silent after. */
const TRAY_EXPLAINED_KEY = "notetaker.trayExplained";
/**
 * Tracks the one-time start-with-Windows default. Two values, not a flag:
 * `pending` means we committed to asking the OS, `done` means the OS said yes.
 * The difference is what lets a refused enable be retried without ever asking
 * twice on a machine whose storage cannot remember the answer.
 */
const AUTOSTART_INIT_KEY = "notetaker.autostartInit";
const AUTOSTART_PENDING = "pending";
const AUTOSTART_DONE = "1";

/** What the hotkeys fall back to when settings cannot be read. */
const DEFAULT_TOGGLE_RECORD = "CommandOrControl+Alt+N";
const DEFAULT_SHOW_HIDE = "CommandOrControl+Alt+Space";
const DEFAULT_HIGHLIGHT = "CommandOrControl+Alt+H";
const DEFAULT_DICTATION = "CommandOrControl+Alt+D";

const EMPTY_DICTATION_STATUS: DictationStatus = {
  state: "idle",
  elapsedS: 0,
  level: 0,
  text: "",
  message: null,
};

/**
 * True while a take is on the line.
 *
 * One definition, used by every path that can end the process, because the
 * cost of two copies disagreeing is a destroyed recording. `finishing` is
 * deliberately *not* live: capture has already stopped and the encoder owns
 * the file, so there is nothing left for a stop-and-save to do.
 */
export function isCapturing(state: CaptureState): boolean {
  return state === "recording" || state === "paused";
}

function isDictating(state: DictationStatus["state"]): boolean {
  return state !== "idle" && state !== "error";
}

function dictationStatusLine(status: DictationStatus): string {
  switch (status.state) {
    case "recording":
      return "Listening — release to transcribe";
    case "transcribing":
      return "Transcribing locally…";
    case "pasting":
      return "Pasting at the active cursor…";
    case "error":
      return status.message ?? "Dictation needs attention.";
    default:
      return status.message ?? "Dictation ready";
  }
}

function captureStatusLine(status: Pick<CaptureStatus, "state" | "elapsedS">): string {
  switch (status.state) {
    case "recording":
      return `Recording — ${duration(status.elapsedS)}`;
    case "paused":
      return `Paused — ${duration(status.elapsedS)}`;
    case "finishing":
      return "Finishing…";
    default:
      return "Not recording";
  }
}

/**
 * Quits the whole app, tray and all.
 *
 * The plugin is imported here rather than at the top of the file for the same
 * reason `updater.ts` does it: it is desktop-only, and a static import pulls it
 * into the served web bundle — which rollup also reports as a warning, because
 * `updater.ts` already asked for it dynamically. The `isDesktop()` check is
 * belt and braces: every caller is already behind one, but a function that can
 * end the process should not rely on its callers for that.
 */
async function quitApp(): Promise<void> {
  if (!isDesktop()) return;
  const { exit } = await import("@tauri-apps/plugin-process");
  await exit(0);
}

function readFirstRunDismissed(): boolean {
  try {
    return window.localStorage.getItem(FIRST_RUN_DISMISSED_KEY) === "1";
  } catch {
    // localStorage can throw (private mode, disabled storage) — degrade to
    // "not dismissed yet" rather than crash the shell over a nagging card.
    return false;
  }
}

/**
 * Which recording is mid-capture, if any.
 *
 * Every non-idle state counts, `finishing` included: until the recording has
 * landed, the file on disk is still moving and should not be played.
 */
export function liveRecordingId(
  status: Pick<CaptureStatus, "state" | "recordingId">,
): string | null {
  return status.state === "idle" ? null : status.recordingId;
}

function App() {
  const lib = useLibrary();
  const capture = useCapture();
  const theme = useTheme();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
  const [processBlocked, setProcessBlocked] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [askOpen, setAskOpen] = useState(false);
  const [firstRunDismissed, setFirstRunDismissed] = useState(readFirstRunDismissed);
  const [modelsMissing, setModelsMissing] = useState(false);
  const [appSettings, setAppSettings] = useState<SettingsData | null>(null);
  // Whether that fetch has *finished*, which is not the same as whether it
  // produced anything — see the effect below.
  const [settingsSettled, setSettingsSettled] = useState(false);
  // Bumped by Settings on every successful write. See the refetch below.
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [showTrayNote, setShowTrayNote] = useState(false);
  const [showQuitGuard, setShowQuitGuard] = useState(false);
  const [inputDevices, setInputDevices] = useState<InputDevice[]>([]);
  const [modelState, setModelState] = useState<TrayModelState>("sleeping");
  const [highlights, setHighlights] = useState<string[]>([]);
  const [dictationStatus, setDictationStatus] = useState<DictationStatus>(EMPTY_DICTATION_STATUS);

  const appSettingsRef = useRef(appSettings);
  appSettingsRef.current = appSettings;
  const libraryRef = useRef(lib);
  libraryRef.current = lib;
  const dictationStatusRef = useRef(dictationStatus);
  dictationStatusRef.current = dictationStatus;

  useEffect(() => {
    if (!isDesktop()) return;
    void listInputDevices().then((devices) => setInputDevices(devices ?? []));
  }, []);

  // Keep installed copies current without ever restarting during a recording.
  // The updater itself verifies the signed artifact before installation.
  useAutoUpdate(capture.status.state === "idle");

  // Dictation has its own fast status loop: the waveform must reflect the
  // gated microphone level while the main capture poll remains on its normal
  // cadence. A failed poll is retained as an honest message rather than
  // making a live run appear idle.
  useEffect(() => {
    if (!isDesktop()) return;
    let cancelled = false;
    const poll = () => {
      void api
        .dictationStatus()
        .then((status) => {
          // Older desktop shells do not expose the dictation status command;
          // an empty IPC response must not erase the safe idle state.
          if (!cancelled && status) setDictationStatus(status);
        })
        .catch((error) => {
          if (cancelled || dictationStatusRef.current.state === "idle") return;
          setDictationStatus((current) => ({
            ...current,
            state: "error",
            message: `Dictation status is unavailable: ${error instanceof Error ? error.message : String(error)}`,
          }));
        });
    };
    poll();
    const timer = window.setInterval(poll, 100);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  // The tray icon is the only thing that says "still recording" once the window
  // is hidden, so it follows capture state rather than being set at start/stop
  // — a session that ends itself never passes through a button handler.
  // The status line rides along: elapsed time comes from the same 1s status
  // poll the record bar reads, so the menu's first line stays current without
  // a timer of its own.
  useEffect(() => {
    const s = capture.status;
    void setTrayStatus(s.state, captureStatusLine(s));
  }, [capture.status]);

  // The floating overlay follows the same principle as the tray: this window
  // owns all state and pushes a rendering, the pill answers with intents.
  // Visibility is policy from settings — "recording" shows it with each
  // recording; "meeting" also shows it as the record-this? prompt the moment
  // the watcher reports a meeting. Fired from the same 1s status poll, so the
  // pill's clock ticks without a timer of its own.
  useEffect(() => {
    if (!isDesktop()) return;
    const mode = appSettings?.overlay ?? "recording";
    const s = capture.status;
    const capturing = isCapturing(s.state) || s.state === "finishing";
    const prompting = mode === "meeting" && !capturing && capture.pendingMeeting != null;
    const dictating = mode !== "off" && isDictating(dictationStatus.state);
    const visible = mode !== "off" && (capturing || prompting || dictating);

    void (async () => {
      try {
        const overlay = await WebviewWindow.getByLabel("overlay");
        if (!overlay) return;
        if (visible) {
          const appName = prompting ? capture.pendingMeeting?.appName ?? null : capture.activeAppName;
          let liveTranscript: LiveTranscriptEvent[] = [];
          if (capturing) {
            try {
              liveTranscript = await api.liveTranscript();
            } catch {
              // The recording remains useful when an older shell has not
              // shipped the optional live-transcript command yet.
            }
          }
          const monitor = await primaryMonitor();
          if (monitor) {
            const size = monitor.size;
            const scale = monitor.scaleFactor;
            const logicalWidth = size.width / scale;
            const logicalHeight = size.height / scale;
            const x = dictating ? (logicalWidth - 300) / 2 : logicalWidth - 300 - 16;
            const y = dictating ? logicalHeight - 78 : 48;
            await overlay.setPosition(new LogicalPosition(x, y));
          }
          await emit("overlay-sync", {
            kind: dictating ? "dictation" : prompting ? "prompt" : "recording",
            state: dictating ? "recording" : s.state === "paused" ? "paused" : s.state === "finishing" ? "finishing" : "recording",
            elapsedS: dictating ? dictationStatus.elapsedS : s.elapsedS,
            recordingId: prompting || dictating ? null : s.recordingId,
            micLevel: dictating ? dictationStatus.level : s.micLevel,
            systemLevel: dictating ? 0 : s.systemLevel,
            appName,
            highlights,
            statusLine: dictating ? dictationStatusLine(dictationStatus) : prompting ? "Meeting detected" : captureStatusLine(s),
            liveTranscript,
            style: appSettings?.overlayStyle ?? "glass",
            dictationState: dictating ? dictationStatus.state : undefined,
            dictationText: dictating ? dictationStatus.text : undefined,
            dictationMessage: dictating ? dictationStatus.message : undefined,
          });
          if (!(await overlay.isVisible())) await overlay.show();
        } else if (await overlay.isVisible()) {
          await overlay.hide();
        }
      } catch {
        // No overlay window (older shell, or the served UI): the app works
        // identically without the pill.
      }
    })();
  }, [capture.status, capture.pendingMeeting, capture.activeAppName, appSettings?.overlay, appSettings?.overlayStyle, highlights, dictationStatus]);

  // The shell starts the overlay protected, then follows the saved preference
  // once Settings has loaded. This is a real runtime toggle on platforms that
  // support it; macOS 15.4+ can still ignore the request, which Settings says
  // plainly instead of promising invisibility.
  useEffect(() => {
    if (!isDesktop()) return;
    void WebviewWindow.getByLabel("overlay")
      .then((overlay) => overlay?.setContentProtected(appSettings?.overlayHideFromShare ?? true))
      .catch(() => undefined);
  }, [appSettings?.overlayHideFromShare]);

  // Push the same owner snapshot to the tray panel whenever its visible state
  // changes. The panel's ready event below covers the race where it mounts
  // after this effect has already fired.
  useEffect(() => {
    if (!isDesktop()) return;
    const snapshot = traySyncRef.current;
    if (snapshot) void emit("tray-panel-sync", snapshot).catch(() => {});
  }, [capture.status, capture.activeAppName, lib.recordings, inputDevices, appSettings?.inputDevice, modelState]);

  // Loaded for the sidebar's empty-state hotkey hint, and to decide which
  // accelerators are registered OS-wide. Refetched on `settingsVersion`, which
  // Settings bumps the moment a write lands — *not* only when the panel closes.
  // A rebind has to take effect while you are still looking at the field:
  // otherwise the old accelerator stays live, the new one does nothing, and if
  // the new one is already taken by another app that message does not appear
  // until Settings is opened a second time. Guarded against the stale-response
  // race, because this effect fires on mount, on open, on close, and on every
  // save, so a slow earlier request could otherwise resolve after a newer one
  // and clobber fresh state with stale state.
  //
  // `settingsSettled` flips on *both* outcomes on purpose. The hotkeys hang off
  // it, and gating them on `appSettings !== null` instead would mean that a
  // getSettings failure leaves them permanently unregistered *and* silent —
  // exactly the "quietly does nothing" failure the hook exists to prevent. A
  // failed read costs the custom bindings, not the hotkeys: the defaults below
  // register anyway, and any conflict still reaches Settings.
  useEffect(() => {
    let ignore = false;
    api
      .getSettings()
      .then((settings) => {
        if (!ignore) {
          setAppSettings(settings);
          setSettingsSettled(true);
        }
      })
      .catch(() => {
        // Deliberately keeps the last good value. Nulling it here would fall
        // through to the default accelerators below, so one flaky read would
        // silently stop the combination the user chose from working and start
        // one they never asked for — with nothing said. A first-load failure
        // still gets the defaults, because `appSettings` starts null anyway.
        if (!ignore) setSettingsSettled(true);
      });
    return () => {
      ignore = true;
    };
  }, [settingsOpen, settingsVersion]);

  // Stable, because Settings takes it as a prop and threads it through a
  // `useCallback`; a new function each render would churn that callback.
  const noteSettingsSaved = useCallback(() => setSettingsVersion((v) => v + 1), []);

  const observeSetupStatus = useCallback((setup: SetupStatus | null) => {
    setModelsMissing((setup?.missing.length ?? 0) > 0);
  }, []);

  // Every path that opens Settings through this helper lands on General. The
  // one exception is the command palette's deep links, which call
  // setSettingsSection directly with a specific section instead of going
  // through here.
  const openSettingsPanel = useCallback(() => {
    setSettingsSection("general");
    setSettingsOpen(true);
  }, []);

  function dismissFirstRun() {
    setFirstRunDismissed(true);
    try {
      window.localStorage.setItem(FIRST_RUN_DISMISSED_KEY, "1");
    } catch {
      // Best effort only — worst case the card reappears next launch.
    }
  }

  // Cmd+J opens the ask panel on whatever is selected. Ignored when nothing is,
  // rather than opening an empty panel that can only say "pick a recording".
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "j" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        if (lib.selectedId) setAskOpen((o) => !o);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [lib.selectedId]);

  /**
   * Stopping a recording lands a new one in the library and opens it, so the
   * notes you typed while it ran are still in front of you rather than
   * somewhere in a list.
   */
  const stopAndOpen = useCallback(async () => {
    const id = await capture.stop();
    await lib.refreshRecordings();
    if (id) await lib.selectRecording(id);
  }, [capture, lib]);

  // Live values for mount-once listeners and OS hotkeys. Without these,
  // effect deps on `capture` would re-run every poll tick.
  const captureRef = useRef(capture);
  captureRef.current = capture;
  const stopAndOpenRef = useRef(stopAndOpen);
  stopAndOpenRef.current = stopAndOpen;
  const closeToTrayRef = useRef(true);
  closeToTrayRef.current = appSettings?.closeToTray ?? true;
  const highlightRecordingRef = useRef<string | null>(null);
  useEffect(() => {
    const recordingId = capture.status.recordingId;
    if (recordingId && recordingId !== highlightRecordingRef.current) {
      highlightRecordingRef.current = recordingId;
      setHighlights([]);
    } else if (!recordingId && capture.status.state === "idle") {
      highlightRecordingRef.current = null;
      setHighlights([]);
    }
  }, [capture.status.recordingId, capture.status.state]);
  const traySyncRef = useRef<TrayPanelSync | null>(null);

  const recentNotes = lib.recordings
    .filter((recording) => recording.hasNotes)
    .slice(0, 5)
    .map((recording) => ({
      id: recording.id,
      title: recording.title,
      created: recording.created,
      durationS: recording.durationS,
    }));
  traySyncRef.current = {
    capture: {
      state: capture.status.state,
      mode: capture.status.mode,
      recordingId: capture.status.recordingId,
      elapsedS: capture.status.elapsedS,
      micLevel: capture.status.micLevel,
      systemLevel: capture.status.systemLevel,
      appName: capture.activeAppName,
    },
    recentNotes,
    inputDevices,
    selectedInputDevice: appSettings?.inputDevice ?? null,
    modelState,
    statusLine: captureStatusLine(capture.status),
  };

  /**
   * Start or stop, whichever the current state calls for.
   *
   * One function for the tray's toggle and the OS-wide record hotkey, because
   * they are the same decision. Identity-stable — every live value arrives
   * through a ref — because `useGlobalHotkeys` depends on it, and a new
   * function on every one-second status poll would unregister and re-register
   * the OS shortcut once a second.
   */
  const toggleRecording = useCallback(() => {
    const c = captureRef.current;
    if (isCapturing(c.status.state)) {
      void stopAndOpenRef.current();
    } else if (c.status.state === "idle") {
      c.start("meeting", "");
    } // finishing: ignore — the recording is still landing.
  }, []);

  /**
   * Stars the current moment of the live recording and keeps the owner's
   * rendering snapshot in step for both floating remotes. The file write is
   * append-only in core; a failed star is intentionally quiet for a stray
   * hotkey press when nothing is recording.
   */
  const addHighlight = useCallback(async () => {
    try {
      const line = await api.addHighlight();
      setHighlights((current) => [...current, line]);
    } catch {
      // Nothing is recording, or the append failed. The native menu and pill
      // stay usable without surfacing a transient error dialog.
    }
  }, []);
  const addHighlightRef = useRef(addHighlight);
  addHighlightRef.current = addHighlight;
  const starMoment = useCallback(() => {
    void addHighlightRef.current();
  }, []);

  const showDictationError = useCallback((error: unknown) => {
    setDictationStatus({
      ...EMPTY_DICTATION_STATUS,
      state: "error",
      message: error instanceof Error ? error.message : String(error),
    });
  }, []);

  const startDictation = useCallback(() => {
    if (isDictating(dictationStatusRef.current.state)) return;
    void api.startDictation().then(setDictationStatus).catch(showDictationError);
  }, [showDictationError]);

  const stopDictation = useCallback(() => {
    if (!isDictating(dictationStatusRef.current.state)) return;
    void api.stopDictation().then(setDictationStatus).catch(showDictationError);
  }, [showDictationError]);

  const cancelDictation = useCallback(() => {
    if (!isDictating(dictationStatusRef.current.state)) return;
    void api.cancelDictation().then(setDictationStatus).catch(showDictationError);
  }, [showDictationError]);

  const toggleDictation = useCallback(() => {
    if (isDictating(dictationStatusRef.current.state)) stopDictation();
    else startDictation();
  }, [startDictation, stopDictation]);

  const hotkeys = useGlobalHotkeys({
    enabled: settingsSettled,
    toggleRecord: appSettings?.hotkeyToggleRecord ?? DEFAULT_TOGGLE_RECORD,
    showHide: appSettings?.hotkeyShowHide ?? DEFAULT_SHOW_HIDE,
    highlight: appSettings?.hotkeyHighlight ?? DEFAULT_HIGHLIGHT,
    onToggleRecord: toggleRecording,
    onHighlight: starMoment,
    dictationHotkey: appSettings?.dictationHotkey ?? DEFAULT_DICTATION,
    dictationMode: appSettings?.dictationMode ?? "pushToTalk",
    dictating: isDictating(dictationStatus.state),
    onDictationStart: startDictation,
    onDictationStop: stopDictation,
    onDictationToggle: toggleDictation,
    onDictationCancel: cancelDictation,
  });

  // Start with Windows, on by default — asked for exactly once, ever.
  //
  // Two invariants, and they pull in opposite directions:
  //
  //   1. Never ask twice on a store that cannot remember the answer. Asking on
  //      every launch would re-enable a login item somebody deliberately
  //      turned off in Settings, and they would have no way to make it stick.
  //   2. A refused `enable()` must be retried, or a single bad launch leaves
  //      start-with-Windows off forever with the app believing it did the job.
  //
  // Both are satisfied by writing the marker in two stages. The first write is
  // also the writability probe: a store that refuses stops us *here*, before
  // the OS is touched at all, which is invariant 1. It is only promoted to
  // `AUTOSTART_DONE` once the OS has actually accepted, so a refusal leaves it
  // at `AUTOSTART_PENDING` and the next launch asks again — invariant 2.
  useEffect(() => {
    if (!isDesktop()) return;
    try {
      if (window.localStorage.getItem(AUTOSTART_INIT_KEY) === AUTOSTART_DONE) return;
      window.localStorage.setItem(AUTOSTART_INIT_KEY, AUTOSTART_PENDING);
    } catch {
      // Storage refused. Doing nothing is the safe half of this: better to
      // never turn it on than to turn it back on at every launch.
      return;
    }
    void (async () => {
      if (!(await setAutostart(true))) return;
      try {
        window.localStorage.setItem(AUTOSTART_INIT_KEY, AUTOSTART_DONE);
      } catch {
        // Near-impossible: the probe above already succeeded on this same key.
        // If it somehow happens the marker stays pending and we ask again next
        // launch, which is the redundant direction rather than the harmful one.
      }
    })();
  }, []);

  // What the native shell asks the webview to decide: whether closing the
  // window means hide or quit, and what the tray's one toggle should do.
  useEffect(() => {
    if (!isDesktop()) return;
    const unlistens = [
      listen("close-requested", async () => {
        if (!closeToTrayRef.current) {
          if (isCapturing(captureRef.current.status.state)) {
            // Never let quit eat a take: stop-and-save is offered first.
            setShowQuitGuard(true);
            return;
          }
          await quitApp();
          return;
        }
        let explained = false;
        try {
          explained = window.localStorage.getItem(TRAY_EXPLAINED_KEY) === "1";
        } catch {
          // Storage can refuse; explaining twice beats crashing on close.
        }
        if (!explained) {
          setShowTrayNote(true);
          return;
        }
        await getCurrentWindow().hide();
      }),
      listen("tray-toggle-recording", () => toggleRecording()),
      // The tray's own controls. None of these shows the window — the point
      // of the tray is doing this without opening the app. Each one re-reads
      // live state from the ref: a menu can sit open while the state changes
      // under it, so the item pressed may no longer apply, and doing nothing
      // is better than doing the wrong thing.
      listen<string>("tray-record", (e) => {
        const c = captureRef.current;
        if (c.status.state === "idle") {
          c.start(e.payload === "in_person" ? "in_person" : "meeting", "");
        }
      }),
      listen("tray-pause-resume", () => {
        const c = captureRef.current;
        if (c.status.state === "recording") c.pause();
        else if (c.status.state === "paused") c.resume();
      }),
      listen("tray-stop", () => {
        if (isCapturing(captureRef.current.status.state)) void stopAndOpenRef.current();
      }),
      listen("tray-highlight", () => void addHighlightRef.current()),
      listen("tray-copy-last-transcript", () => {
        void api
          .copyLastTranscript()
          .then((result) => setDictationStatus((current) => ({ ...current, message: result.message })))
          .catch(showDictationError);
      }),
      listen("tray-open", async () => {
        const win = getCurrentWindow();
        await win.show();
        await win.unminimize();
        await win.setFocus();
      }),
      listen<string>("tray-open-note", async (event) => {
        await libraryRef.current.selectRecording(event.payload);
        const win = getCurrentWindow();
        await win.show();
        await win.unminimize();
        await win.setFocus();
      }),
      listen<string | null>("tray-mic-select", async (event) => {
        const current = appSettingsRef.current;
        if (!current) return;
        const inputDevice = event.payload || null;
        const priority = inputDevice
          ? [inputDevice, ...current.audioDevicePriority.filter((id) => id !== inputDevice)]
          : current.audioDevicePriority;
        const next = { ...current, inputDevice, audioDevicePriority: priority };
        try {
          await api.setSettings(next);
          setAppSettings(next);
          setSettingsVersion((version) => version + 1);
        } catch {
          // The panel remains usable; Settings will show the previous value on
          // its next open if the native write was refused.
        }
      }),
      listen("tray-panel-ready", () => {
        const snapshot = traySyncRef.current;
        if (snapshot) void emit("tray-panel-sync", snapshot).catch(() => {});
      }),
      listen<{ state?: string }>("model-state-changed", (event) => {
        switch (event.payload?.state) {
          case "ready":
            setModelState("ready");
            break;
          case "loading":
            setModelState("loading");
            break;
          case "failed":
            setModelState("error");
            break;
          default:
            setModelState("sleeping");
            break;
        }
      }),
      // The overlay pill's intents — same owners as the tray's, plus the
      // prompt pair. `overlay-record` prefers the pending meeting (it carries
      // the app's name for the title) and falls back to a plain meeting
      // recording so the button still works if the prompt expired underneath.
      listen("overlay-record", () => {
        const c = captureRef.current;
        if (c.pendingMeeting) void c.recordPendingMeeting();
        else if (c.status.state === "idle") c.start("meeting", "");
      }),
      listen("overlay-dismiss", () => captureRef.current.dismissPendingMeeting()),
      listen("overlay-highlight", () => void addHighlightRef.current()),
      listen("overlay-pause-resume", () => {
        const c = captureRef.current;
        if (c.status.state === "recording") c.pause();
        else if (c.status.state === "paused") c.resume();
      }),
      listen("overlay-stop", () => {
        if (isCapturing(captureRef.current.status.state)) void stopAndOpenRef.current();
      }),
      listen("overlay-dictation-stop", () => stopDictation()),
      listen("overlay-dictation-cancel", () => cancelDictation()),
      listen("overlay-open", async () => {
        const win = getCurrentWindow();
        await win.show();
        await win.unminimize();
        await win.setFocus();
      }),
      listen<{ recordingId?: string; text?: string }>("overlay-jot", async (event) => {
        const recordingId = event.payload?.recordingId;
        const text = event.payload?.text;
        if (!recordingId || !text?.trim()) return;
        try {
          await api.appendNote(recordingId, text);
          await libraryRef.current.refreshRecordings();
        } catch {
          // The overlay explains the append-only behavior; a failed write does
          // not erase the draft in its own textarea.
        }
      }),
      listen("tray-open-settings", async () => {
        const win = getCurrentWindow();
        await win.show();
        await win.unminimize();
        await win.setFocus();
        setSettingsOpen(true);
      }),
      // The tray's Quit asks rather than exits. It used to call `app.exit(0)`
      // straight from the menu handler, which skips destructors — so quitting
      // mid-recording lost the last unflushed buffer and left the take to be
      // picked up as a crash recovery on the next launch. Same guard as the
      // close button now, and the same one dialog.
      listen("tray-quit-requested", async () => {
        if (isCapturing(captureRef.current.status.state)) {
          setShowQuitGuard(true);
          return;
        }
        await quitApp();
      }),
    ];
    // A `listen` that never resolves its subscription must not surface as an
    // unhandled rejection — the window still has to close either way.
    unlistens.forEach((p) => p.catch(() => {}));
    return () => {
      unlistens.forEach((p) => p.then((u) => u()).catch(() => {}));
    };
    // Mounts once: `toggleRecording` is identity-stable and every other live
    // value is read through a ref above. Depending on `capture` here would
    // tear the listeners down and rebuild them on every one-second poll.
  }, [toggleRecording, showDictationError, stopDictation, cancelDictation]);

  // "Process now" queues the recording and wakes the scheduler. When there is
  // no scheduler — because the speech models were never downloaded — that call
  // still succeeds, the row flips to "queued", and it stays there forever. It
  // did exactly that on a real machine for three recordings without one word of
  // explanation. So: ask first, and say the true thing instead.
  const processNow = useCallback(
    async (id: string) => {
      const setup = await api.setupStatus().catch(() => null);
      if (setup && !setup.transcribing) {
        setProcessBlocked(
          setup.missing.length > 0
            ? "Nothing can be transcribed yet — the speech models aren't downloaded. This recording is safe and will be processed as soon as they are."
            : "Transcription isn't running. Your recording is safe; restarting the app usually fixes this.",
        );
      }
      await api.processNow(id);
      await lib.refreshRecordings();
    },
    [lib],
  );

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col overflow-hidden bg-app text-fg">
        {/*
          The titlebar, drawn by the app.

          `decorations: false` in tauri.conf.json means this strip is the only
          titlebar there is, so it has to do a titlebar's jobs — move the
          window, and maximise it on a double-click — as well as its own. Both
          come from `data-tauri-drag-region`, and the value matters. Tauri's
          injected script walks the event path upward from whatever was
          pressed: `"deep"` means anything in this subtree drags the window,
          where a bare attribute would mean "only a direct hit on the header
          element itself" and leave the recording pill and the wordmark dead to
          a drag. The same walk stops at the first `<button>`, `<a>`, `<input>`,
          `<label>` or `role="button"` it meets, so every control in here — the
          record button, the mode picker, pause, stop, the two icon buttons and
          the three window controls — presses instead of dragging, and none of
          them has to opt out by hand.

          There is deliberately no `onDoubleClick` here. That same script
          already toggles maximise on a double-click of a drag region; a React
          handler on top of it would fire second and toggle straight back, so
          the titlebar would appear to ignore double-clicks entirely.

          `pr-0` so Close sits flush in the corner, where a thrown mouse lands.
        */}
        <header
          data-tauri-drag-region="deep"
          className={cn(
            "flex shrink-0 items-center justify-between gap-3 border-b border-border bg-raised/80 py-1.5",
            // macOS keeps its native traffic lights, overlaid top-left on this
            // strip — so the record bar steps right to clear them, and with no
            // custom controls in the corner the flush `pr-0` has no job.
            isMacDesktop() ? "pl-[84px] pr-2" : "pl-3 pr-0",
          )}
        >
          <RecordBar
            status={capture.status}
            onStart={capture.start}
            onPause={capture.pause}
            onResume={capture.resume}
            onStop={stopAndOpen}
          />
          {/* Hidden on a phone, where the rail's own header already names the
              app and 12px of tracking-out capitals would only crowd the pill. */}
          <span className="pointer-events-none hidden select-none text-[12px] font-semibold tracking-[0.08em] text-fg-faint sm:block">
            NOTETAKER
          </span>
          <div className="flex items-center gap-1">
            <IconButton
              label={theme.resolved === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              onClick={theme.toggle}
            >
              {theme.resolved === "dark" ? <Sun size={15} /> : <Moon size={15} />}
            </IconButton>
            <IconButton label="Settings" onClick={openSettingsPanel}>
              <SettingsIcon size={15} />
            </IconButton>
            <WindowControls />
          </div>
        </header>

        {capture.captureError && (
          <Notice className="mx-3 mt-2 shrink-0">{capture.captureError}</Notice>
        )}
        {dictationStatus.message && (
          <Notice className="mx-3 mt-2 shrink-0">{dictationStatus.message}</Notice>
        )}
        <SetupNotice onOpenSettings={openSettingsPanel} onStatus={observeSetupStatus} />
        {processBlocked && (
          <Notice className="mx-3 mt-2 shrink-0">
            {processBlocked}{" "}
            <button
              type="button"
              onClick={() => setProcessBlocked(null)}
              className="underline underline-offset-2"
            >
              Dismiss
            </button>
          </Notice>
        )}
        {lib.loadError && (
          <Notice className="mx-3 mt-2 shrink-0">
            {lib.loadError}{" "}
            <button
              type="button"
              onClick={lib.dismissError}
              className="underline underline-offset-2"
            >
              Dismiss
            </button>
          </Notice>
        )}

        {/*
          Two panes on a desktop, two *screens* on a phone.
          The rail is 264px of a 1440px window and 63% of a 420px one, so below
          `md` whichever of the two you are not looking at is removed rather
          than squeezed — a note in a 155px column is not a note. Which one
          shows is driven by whether a recording is open, so the back button is
          just "close the recording".
        */}
        <div className="flex min-h-0 flex-1">
          <Sidebar
            className={lib.selectedId ? "hidden md:flex" : "flex w-full md:w-[264px]"}
            tasks={lib.tasks}
            activeView={lib.view}
            onSelectView={lib.setView}
            onCreateTask={lib.createTask}
            recordings={lib.recordings}
            selectedId={lib.selectedId}
            onSelectRecording={lib.selectRecording}
            query={lib.query}
            onSearch={lib.search}
            searchResults={lib.searchResults}
            onOpenPalette={() => setPaletteOpen(true)}
            sort={lib.sort}
            onSetSort={lib.setSort}
            filter={lib.filter}
            onSetFilter={lib.setFilter}
            modelsMissing={modelsMissing}
            recordHotkeyLabel={
              isDesktop() && appSettings
                ? formatAcceleratorParts(appSettings.hotkeyToggleRecord).join("+")
                : null
            }
          />

          <main className={lib.selectedId ? "flex min-w-0 flex-1" : "hidden min-w-0 flex-1 md:flex"}>
            <NoteView
              detail={lib.detail}
              loading={lib.detailLoading}
              liveRecordingId={liveRecordingId(capture.status)}
              onBack={lib.clearSelection}
              tasks={lib.tasks}
              templates={lib.templates}
              askOpen={askOpen}
              onToggleAsk={setAskOpen}
              onRenameSpeaker={lib.renameSpeaker}
              onSaveSummary={lib.saveSummary}
              onRenameRecording={lib.renameRecording}
              onAssignTask={lib.assignTask}
              onArchiveRecording={lib.archiveRecording}
              onRestoreRecording={lib.restoreRecording}
              onDeleteRecording={lib.deleteRecording}
              onSaveNotes={lib.saveNotes}
              onSetTemplate={lib.setTemplate}
              onToggleAction={lib.toggleAction}
              onProcessNow={processNow}
            />
          </main>
        </div>

        <CommandPalette
          open={paletteOpen}
          onOpenChange={setPaletteOpen}
          recordings={lib.recordings}
          tasks={lib.tasks}
          onSelectRecording={lib.selectRecording}
          onSelectTask={(name) => lib.setView({ kind: "task", name })}
          onOpenSettings={(section) => {
            setSettingsSection(section ?? "general");
            setSettingsOpen(true);
          }}
        />

        {capture.pendingMeeting && (
          <MeetingPrompt
            event={capture.pendingMeeting}
            onRecord={capture.recordPendingMeeting}
            onNotNow={capture.dismissPendingMeeting}
            onAlways={capture.alwaysRecordPending}
            onNever={capture.neverRecordPending}
          />
        )}

        {settingsOpen && (
          <Settings
            onClose={() => setSettingsOpen(false)}
            theme={theme}
            section={settingsSection}
            onSelectSection={setSettingsSection}
            hotkeyIssues={hotkeys.issues}
            onSaved={noteSettingsSaved}
          />
        )}

        <Dialog
          open={showTrayNote}
          onOpenChange={(o) => setShowTrayNote(o)}
          title="Still running"
          description="Notetaker keeps running here in the tray so meeting detection and your recording hotkey still work. Quit completely from the tray icon."
        >
          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              onClick={async () => {
                // This note is the *first* close of a fresh install, and it is
                // shown before anything has looked at the recording. Quitting
                // straight from here would end a live take with nothing saved,
                // so the guard has to be asked here too, not only on the path
                // that skips this dialog.
                if (isCapturing(captureRef.current.status.state)) {
                  setShowTrayNote(false);
                  setShowQuitGuard(true);
                  return;
                }
                await quitApp();
              }}
            >
              Quit instead
            </Button>
            <Button
              variant="primary"
              onClick={async () => {
                try {
                  window.localStorage.setItem(TRAY_EXPLAINED_KEY, "1");
                } catch {
                  // Best effort only — worst case the note appears again.
                }
                setShowTrayNote(false);
                await getCurrentWindow().hide();
              }}
            >
              Got it
            </Button>
          </div>
        </Dialog>
        <Dialog
          open={showQuitGuard}
          onOpenChange={setShowQuitGuard}
          title="Recording in progress"
          description="Quitting now would end the recording. It will be stopped and saved first."
        >
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setShowQuitGuard(false)}>
              Keep recording
            </Button>
            <Button
              variant="primary"
              onClick={async () => {
                await stopAndOpenRef.current();
                await quitApp();
              }}
            >
              Stop and save, then quit
            </Button>
          </div>
        </Dialog>

        {!firstRunDismissed && <FirstRun onDismiss={dismissFirstRun} />}
      </div>
    </TooltipProvider>
  );
}

export default App;
