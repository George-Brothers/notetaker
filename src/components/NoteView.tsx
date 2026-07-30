/**
 * The note — the main pane, and the thing this app is for.
 *
 * The shape is Granola's: your own notes at the top in full contrast, the
 * model's expansion of them below in grey, the transcript behind a tab rather
 * than in your face. What you wrote is the document; what the AI added is
 * support for it.
 *
 * The title's edit-in-place keeps a `useRef` flag rather than reading state,
 * because Escape and Enter both unmount the focused input and therefore fire a
 * blur — and a blur handler reading stale state is exactly how a *cancelled*
 * rename gets saved anyway.
 */

import { useEffect, useRef, useState } from "react";
import {
  Check,
  FileAudio,
  Loader2,
  NotebookPen,
  RefreshCw,
  Sparkles,
  Wand2,
  X,
} from "lucide-react";
import type { RecordingDetail as RecordingDetailData, Template } from "../lib/ipc";
import { fullDateTime, roughDuration } from "../lib/format";
import { cn } from "../lib/cn";
import { StatusChip } from "./StatusChip";
import { Notepad, type SaveState } from "./Notepad";
import { Markdown } from "./Markdown";
import { ActionItems } from "./ActionItems";
import { TranscriptPanel } from "./TranscriptPanel";
import { AskPanel } from "./AskPanel";
import {
  Button,
  IconButton,
  Notice,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Tab,
  TabList,
  TabPanel,
  Tabs,
  Tip,
} from "./ui";

export interface NoteViewProps {
  detail: RecordingDetailData | null;
  loading: boolean;
  tasks: string[];
  templates: Template[];
  askOpen: boolean;
  onToggleAsk: (open: boolean) => void;
  onRenameSpeaker: (id: string, key: string, name: string) => void;
  onSaveSummary: (id: string, summaryMd: string) => void;
  onRenameRecording: (id: string, title: string) => void;
  onAssignTask: (id: string, task: string) => void;
  onSaveNotes: (id: string, notesMd: string) => Promise<void>;
  onSetTemplate: (id: string, template: string) => void;
  onToggleAction: (id: string, index: number, done: boolean) => void;
  onProcessNow: (id: string) => void;
}

function SaveIndicator({ state }: { state: SaveState }) {
  if (state === "idle") return null;
  return (
    <span className="flex items-center gap-1 text-[11px] text-fg-faint" role="status">
      {state === "saving" ? (
        <>
          <Loader2 size={10} className="animate-spin" aria-hidden />
          Saving
        </>
      ) : (
        <>
          <Check size={10} aria-hidden />
          Saved
        </>
      )}
    </span>
  );
}

export function NoteView({
  detail,
  loading,
  tasks,
  templates,
  askOpen,
  onToggleAsk,
  onRenameSpeaker,
  onSaveSummary,
  onRenameRecording,
  onAssignTask,
  onSaveNotes,
  onSetTemplate,
  onToggleAction,
  onProcessNow,
}: NoteViewProps) {
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  // What the user just renamed this to, shown until the host refetches —
  // without it the heading snaps back and the rename looks like it failed.
  const [savedTitle, setSavedTitle] = useState<string | null>(null);
  const titleEditRef = useRef(false);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [editingSummary, setEditingSummary] = useState(false);
  const [summaryDraft, setSummaryDraft] = useState("");

  useEffect(() => {
    titleEditRef.current = false;
    setEditingTitle(false);
    setSavedTitle(null);
    setSaveState("idle");
    setEditingSummary(false);
  }, [detail?.id]);

  useEffect(() => {
    setSummaryDraft(detail?.summaryMd ?? "");
  }, [detail?.id, detail?.summaryMd]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center" aria-live="polite">
        <span className="flex items-center gap-2 text-[13px] text-fg-muted">
          <Loader2 size={14} className="animate-spin" aria-hidden />
          Loading recording…
        </span>
      </div>
    );
  }

  if (!detail) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
        <NotebookPen size={22} className="text-fg-faint" aria-hidden />
        <p className="text-sm font-medium text-fg">Pick a recording, or start a new one</p>
        <p className="max-w-sm text-[13px] leading-relaxed text-fg-muted">
          Type your own notes while it records. When it finishes, the AI expands what you wrote
          using what it heard.
        </p>
      </div>
    );
  }

  const rec = detail;
  const title = savedTitle ?? rec.title;
  const processed = rec.status === "ready";
  const activeTemplate = templates.find((t) => t.id === (rec.template ?? "default"));

  function beginTitleEdit() {
    titleEditRef.current = true;
    setTitleDraft(title);
    setEditingTitle(true);
  }

  function endTitleEdit() {
    titleEditRef.current = false;
    setEditingTitle(false);
  }

  function commitTitle() {
    if (!titleEditRef.current) return;
    endTitleEdit();
    const trimmed = titleDraft.trim();
    if (!trimmed || trimmed === title) return;
    setSavedTitle(trimmed);
    onRenameRecording(rec.id, trimmed);
  }

  function acceptSuggestedTitle() {
    if (!rec.suggestedTitle) return;
    setSavedTitle(rec.suggestedTitle);
    onRenameRecording(rec.id, rec.suggestedTitle);
  }

  return (
    <div className="flex h-full min-w-0 flex-1">
      <article
        aria-label="Recording"
        className="min-w-0 flex-1 overflow-y-auto"
      >
        <div className="mx-auto w-full max-w-[46rem] px-8 py-8">
          {/* --- header ------------------------------------------------- */}
          <header className="mb-5">
            {editingTitle ? (
              <input
                autoFocus
                aria-label="Recording title"
                value={titleDraft}
                onChange={(e) => setTitleDraft(e.target.value)}
                onBlur={commitTitle}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitTitle();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    endTitleEdit();
                  }
                }}
                className="w-full border-0 border-b border-accent bg-transparent p-0 pb-1 text-[26px] font-semibold leading-tight text-fg focus:outline-none"
              />
            ) : (
              <button
                type="button"
                onClick={beginTitleEdit}
                title="Click to rename"
                className="-mx-1 block w-full rounded px-1 text-left text-[26px] font-semibold leading-tight text-fg hover:bg-hover"
              >
                {title}
              </button>
            )}

            {rec.suggestedTitle && rec.suggestedTitle !== title && (
              <div className="mt-2 flex flex-wrap items-center gap-2 rounded-[var(--radius-control)] bg-accent-soft px-2.5 py-1.5">
                <Sparkles size={13} className="shrink-0 text-accent" aria-hidden />
                <span className="text-[13px] text-fg">
                  Suggested title: <strong className="font-semibold">{rec.suggestedTitle}</strong>
                </span>
                <Button size="sm" variant="primary" onClick={acceptSuggestedTitle}>
                  Use it
                </Button>
              </div>
            )}

            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[12px] text-fg-muted">
              <span>{fullDateTime(rec.created)}</span>
              <span aria-hidden>·</span>
              <span>{roughDuration(rec.durationS)}</span>
              <span aria-hidden>·</span>
              <label className="sr-only" htmlFor="note-task">
                Task
              </label>
              <select
                id="note-task"
                value={rec.task ?? ""}
                onChange={(e) => e.target.value && onAssignTask(rec.id, e.target.value)}
                className="rounded border border-border bg-raised px-1.5 py-0.5 text-[12px] text-fg-muted focus:border-accent focus:outline-none"
              >
                <option value="">Unsorted</option>
                {tasks.map((t) => (
                  <option key={t} value={t}>
                    {t}
                  </option>
                ))}
              </select>
              <StatusChip status={rec.status} error={rec.error} />
            </div>

            {rec.suggestedTask && rec.task !== rec.suggestedTask && (
              <div className="mt-2 flex flex-wrap items-center gap-2 rounded-[var(--radius-control)] bg-sunken px-2.5 py-1.5">
                <span className="text-[13px] text-fg-muted">
                  This looks like it belongs to{" "}
                  <strong className="font-semibold text-fg">{rec.suggestedTask}</strong>
                </span>
                <Button size="sm" onClick={() => onAssignTask(rec.id, rec.suggestedTask!)}>
                  File it there
                </Button>
              </div>
            )}

            {rec.captureNote && (
              <Notice tone="warn" className="mt-2">
                {rec.captureNote}
              </Notice>
            )}
          </header>

          {/* --- tabs and toolbar --------------------------------------- */}
          <Tabs defaultValue="notes" className="flex flex-col">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-2 border-b border-border pb-3">
              <TabList>
                <Tab value="notes">
                  <NotebookPen size={13} />
                  Notes
                </Tab>
                <Tab value="transcript">
                  <FileAudio size={13} />
                  Transcript
                </Tab>
              </TabList>

              <div className="flex items-center gap-1.5">
                <Popover>
                  <PopoverTrigger asChild>
                    <Button size="sm" variant="ghost">
                      <Wand2 size={13} />
                      {activeTemplate?.name ?? "General notes"}
                    </Button>
                  </PopoverTrigger>
                  <PopoverContent align="end" className="w-72">
                    <p className="px-2 py-1.5 text-[11px] leading-snug text-fg-faint">
                      Changes the shape of the AI's notes. Takes effect the next time this
                      recording is processed.
                    </p>
                    {templates.map((t) => (
                      <button
                        key={t.id}
                        type="button"
                        onClick={() => onSetTemplate(rec.id, t.id)}
                        className={cn(
                          "flex w-full flex-col items-start gap-0.5 rounded-[var(--radius-control)] px-2 py-1.5 text-left transition-colors hover:bg-hover",
                          t.id === (rec.template ?? "default") && "bg-selected",
                        )}
                      >
                        <span className="text-[13px] font-medium text-fg">{t.name}</span>
                        <span className="text-[12px] leading-snug text-fg-muted">{t.blurb}</span>
                      </button>
                    ))}
                  </PopoverContent>
                </Popover>

                <Tip label={processed ? "Rewrite the AI notes from your notes and the transcript" : "Process this recording now"}>
                  <Button size="sm" variant="ghost" onClick={() => onProcessNow(rec.id)}>
                    <RefreshCw size={13} />
                    {processed ? "Re-enhance" : "Process now"}
                  </Button>
                </Tip>

                <Button
                  size="sm"
                  variant={askOpen ? "primary" : "ghost"}
                  onClick={() => onToggleAsk(!askOpen)}
                  aria-pressed={askOpen}
                >
                  <Sparkles size={13} />
                  Ask
                </Button>
              </div>
            </div>

            <TabPanel value="notes" className="focus:outline-none">
              <Notepad
                recordingId={rec.id}
                initialNotes={rec.notesMd}
                onSave={onSaveNotes}
                onStateChange={setSaveState}
                placeholder="Type your notes here. Rough is fine — the AI fills in the rest from what it heard."
              />
              <div className="flex justify-end pt-1">
                <SaveIndicator state={saveState} />
              </div>

              {(rec.summaryMd.trim() || rec.actions.length > 0) && (
                <>
                  <div className="my-6 flex items-center gap-2">
                    <span className="h-px flex-1 bg-border" />
                    <span className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-fg-faint">
                      <Sparkles size={11} aria-hidden />
                      Enhanced by AI
                    </span>
                    <span className="h-px flex-1 bg-border" />
                  </div>

                  <ActionItems
                    items={rec.actions}
                    onToggle={(index, done) => onToggleAction(rec.id, index, done)}
                  />

                  {editingSummary ? (
                    <div className="flex flex-col gap-2">
                      <textarea
                        autoFocus
                        aria-label="Summary"
                        value={summaryDraft}
                        onChange={(e) => setSummaryDraft(e.target.value)}
                        onBlur={() => {
                          setEditingSummary(false);
                          if (summaryDraft !== rec.summaryMd) onSaveSummary(rec.id, summaryDraft);
                        }}
                        rows={16}
                        className="w-full rounded-[var(--radius-control)] border border-border bg-sunken p-3 font-mono text-[13px] leading-relaxed text-fg focus:border-accent focus:outline-none"
                      />
                      <p className="text-[11px] text-fg-faint">
                        Markdown. Saves when you click away.
                      </p>
                    </div>
                  ) : (
                    <button
                      type="button"
                      onClick={() => setEditingSummary(true)}
                      title="Click to edit"
                      className="-mx-2 block w-full rounded-[var(--radius-control)] px-2 text-left hover:bg-hover"
                    >
                      {/* Checkbox lines are rendered above, where they can be
                          ticked; showing them twice would be confusing. */}
                      <Markdown muted hideTaskItems>
                        {rec.summaryMd}
                      </Markdown>
                    </button>
                  )}
                </>
              )}

              {!rec.summaryMd.trim() && rec.status !== "ready" && (
                <p className="mt-6 rounded-[var(--radius-control)] bg-sunken px-3 py-2.5 text-[13px] leading-relaxed text-fg-muted">
                  {rec.status === "failed"
                    ? "This recording could not be processed. Your notes above are safe — press Process now to try again."
                    : "The AI notes appear here once this recording has been processed. Your own notes are saved either way."}
                </p>
              )}
            </TabPanel>

            <TabPanel value="transcript" className="focus:outline-none">
              <TranscriptPanel
                detail={rec}
                onRenameSpeaker={(key, name) => onRenameSpeaker(rec.id, key, name)}
              />
            </TabPanel>
          </Tabs>
        </div>
      </article>

      {askOpen && (
        <aside
          aria-label="Ask about this recording"
          className="flex w-[340px] shrink-0 flex-col border-l border-border bg-app px-4 py-4"
        >
          <div className="mb-2 flex items-center justify-between">
            <h2 className="flex items-center gap-1.5 text-[13px] font-semibold text-fg">
              <Sparkles size={13} className="text-accent" aria-hidden />
              Ask this recording
            </h2>
            <IconButton label="Close" onClick={() => onToggleAsk(false)}>
              <X size={15} />
            </IconButton>
          </div>
          <div className="min-h-0 flex-1">
            <AskPanel
              recordingId={rec.id}
              canAsk={rec.transcriptMd.trim().length > 0 || rec.summaryMd.trim().length > 0}
            />
          </div>
        </aside>
      )}
    </div>
  );
}
