/**
 * The shell: a quiet top strip, the library rail, and the note.
 *
 * Two panes rather than the three this used to have. The old layout gave a
 * third of the window to a list of recordings and another third to a view
 * picker, which left the note — the only thing anyone is here to read — in a
 * column too narrow to read comfortably. The rail now does both jobs.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
import { Button, Dialog, IconButton, Notice, TooltipProvider } from "./components/ui";
import {
  api,
  type CaptureState,
  type CaptureStatus,
  type Settings as SettingsData,
  type SetupStatus,
} from "./lib/ipc";
import { isDesktop } from "./lib/transport";
import { setAutostart, setTrayStatus } from "./lib/desktop";
import { formatAcceleratorParts } from "./lib/hotkeys";

const FIRST_RUN_DISMISSED_KEY = "notetaker.firstRunDismissed";
/** Set once the tray note has been read, so closing the window is silent after. */
const TRAY_EXPLAINED_KEY = "notetaker.trayExplained";
/** Set the first time we turn on start-with-Windows, so we only ever do it once. */
const AUTOSTART_INIT_KEY = "notetaker.autostartInit";

/** What the hotkeys fall back to when settings cannot be read. */
const DEFAULT_TOGGLE_RECORD = "CommandOrControl+Alt+N";
const DEFAULT_SHOW_HIDE = "CommandOrControl+Alt+Space";

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

  // Keep installed copies current without ever restarting during a recording.
  // The updater itself verifies the signed artifact before installation.
  useAutoUpdate(capture.status.state === "idle");

  // The tray icon is the only thing that says "still recording" once the window
  // is hidden, so it follows capture state rather than being set at start/stop
  // — a session that ends itself never passes through a button handler.
  useEffect(() => {
    void setTrayStatus(capture.status.state);
  }, [capture.status.state]);

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

  const hotkeys = useGlobalHotkeys({
    enabled: settingsSettled,
    toggleRecord: appSettings?.hotkeyToggleRecord ?? DEFAULT_TOGGLE_RECORD,
    showHide: appSettings?.hotkeyShowHide ?? DEFAULT_SHOW_HIDE,
    onToggleRecord: toggleRecording,
  });

  // Start with Windows, on by default — but written exactly once, ever.
  // Writing it on every launch would silently undo turning it off in Settings.
  useEffect(() => {
    if (!isDesktop()) return;
    try {
      if (window.localStorage.getItem(AUTOSTART_INIT_KEY) === "1") return;
    } catch {
      // Storage refused. Doing nothing is the safe half of this: better to
      // never turn it on than to turn it back on at every launch.
      return;
    }
    void (async () => {
      // Ticked off only once the OS has actually accepted it. Marking it done
      // first meant a refused `enable()` — which `setAutostart` swallows — was
      // never retried: the app would have "already done" a thing that in fact
      // never happened, and start-with-Windows would be off forever.
      if (!(await setAutostart(true))) return;
      try {
        window.localStorage.setItem(AUTOSTART_INIT_KEY, "1");
      } catch {
        // Worst case we ask the OS again next launch, which it will accept.
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
      listen("tray-open-settings", () => setSettingsOpen(true)),
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
  }, [toggleRecording]);

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
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-border px-3 py-2">
          <RecordBar
            status={capture.status}
            onStart={capture.start}
            onPause={capture.pause}
            onResume={capture.resume}
            onStop={stopAndOpen}
          />
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
          </div>
        </header>

        {capture.captureError && (
          <Notice className="mx-3 mt-2 shrink-0">{capture.captureError}</Notice>
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
