/**
 * The shell: a quiet top strip, the library rail, and the note.
 *
 * Two panes rather than the three this used to have. The old layout gave a
 * third of the window to a list of recordings and another third to a view
 * picker, which left the note — the only thing anyone is here to read — in a
 * column too narrow to read comfortably. The rail now does both jobs.
 */

import { useCallback, useEffect, useState } from "react";
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
import { FirstRun } from "./components/FirstRun";
import { SetupNotice } from "./components/SetupNotice";
import { CommandPalette } from "./components/CommandPalette";
import { IconButton, Notice, TooltipProvider } from "./components/ui";
import { api, type CaptureStatus, type SetupStatus } from "./lib/ipc";

const FIRST_RUN_DISMISSED_KEY = "notetaker.firstRunDismissed";

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
  const [processBlocked, setProcessBlocked] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [askOpen, setAskOpen] = useState(false);
  const [firstRunDismissed, setFirstRunDismissed] = useState(readFirstRunDismissed);
  const [modelsMissing, setModelsMissing] = useState(false);

  // Keep installed copies current without ever restarting during a recording.
  // The updater itself verifies the signed artifact before installation.
  useAutoUpdate(capture.status.state === "idle");

  const observeSetupStatus = useCallback((setup: SetupStatus | null) => {
    setModelsMissing((setup?.missing.length ?? 0) > 0);
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
            <IconButton label="Settings" onClick={() => setSettingsOpen(true)}>
              <SettingsIcon size={15} />
            </IconButton>
          </div>
        </header>

        {capture.captureError && (
          <Notice className="mx-3 mt-2 shrink-0">{capture.captureError}</Notice>
        )}
        <SetupNotice onOpenSettings={() => setSettingsOpen(true)} onStatus={observeSetupStatus} />
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
          onSelectRecording={lib.selectRecording}
          capture={capture.status}
          themeIsDark={theme.resolved === "dark"}
          canAsk={lib.selectedId !== null}
          actions={{
            startMeeting: () => capture.start("meeting", ""),
            startInPerson: () => capture.start("in_person", ""),
            stop: () => void stopAndOpen(),
            openSettings: () => setSettingsOpen(true),
            toggleTheme: theme.toggle,
            openAsk: () => setAskOpen(true),
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

        {settingsOpen && <Settings onClose={() => setSettingsOpen(false)} />}

        {!firstRunDismissed && <FirstRun onDismiss={dismissFirstRun} />}
      </div>
    </TooltipProvider>
  );
}

export default App;
