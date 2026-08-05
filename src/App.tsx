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
import { setTrayStatus } from "./lib/desktop";
import { formatAcceleratorParts } from "./lib/hotkeys";

const FIRST_RUN_DISMISSED_KEY = "notetaker.firstRunDismissed";
/** Set once the tray note has been read, so closing the window is silent after. */
const TRAY_EXPLAINED_KEY = "notetaker.trayExplained";

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

  // Loaded for the sidebar's empty-state hotkey hint, and for whatever else
  // ends up wanting a native setting later. Refetched when Settings closes so
  // a rebind made in there shows up here without a full reload. Guarded
  // against the stale-response race: this effect re-fires on mount, on open,
  // and on close, so a slow earlier request could otherwise resolve after a
  // newer one and clobber fresh state with stale state.
  useEffect(() => {
    let ignore = false;
    api
      .getSettings()
      .then((settings) => {
        if (!ignore) setAppSettings(settings);
      })
      .catch(() => {
        if (!ignore) setAppSettings(null);
      });
    return () => {
      ignore = true;
    };
  }, [settingsOpen]);

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
      listen("tray-toggle-recording", () => {
        const c = captureRef.current;
        if (isCapturing(c.status.state)) {
          void stopAndOpenRef.current();
        } else if (c.status.state === "idle") {
          c.start("meeting", "");
        } // finishing: ignore — the recording is still landing.
      }),
      listen("tray-open-settings", () => setSettingsOpen(true)),
    ];
    // A `listen` that never resolves its subscription must not surface as an
    // unhandled rejection — the window still has to close either way.
    unlistens.forEach((p) => p.catch(() => {}));
    return () => {
      unlistens.forEach((p) => p.then((u) => u()).catch(() => {}));
    };
    // Mounts once: every live value it needs is read through a ref above.
    // Depending on `capture` here would tear the listeners down and rebuild
    // them on every one-second status poll.
  }, []);

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
