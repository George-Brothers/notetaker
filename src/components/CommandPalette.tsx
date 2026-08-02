/**
 * Cmd+K — the fastest way to anything.
 *
 * Two kinds of row: recordings, and things to do. Both are matched by `cmdk`'s
 * own scorer over the visible text, so there is no separate search index to
 * keep in step with the sidebar's.
 *
 * Deliberately shows only what is already loaded rather than calling `search`.
 * A palette is a navigation aid and has to answer within a keystroke; full-text
 * search is what the sidebar's search field is for, and it says so at the
 * bottom of the list.
 */

import { useEffect, useMemo } from "react";
import { Command } from "cmdk";
import {
  Circle,
  FileText,
  Mic,
  Moon,
  Settings as SettingsIcon,
  Square,
  Sun,
  Sparkles,
} from "lucide-react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import type { CaptureStatus, RecordingRow } from "../lib/ipc";
import { dayLabel, roughDuration } from "../lib/format";

export interface PaletteActions {
  startMeeting: () => void;
  startInPerson: () => void;
  stop: () => void;
  openSettings: () => void;
  toggleTheme: () => void;
  openAsk: () => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  recordings,
  onSelectRecording,
  capture,
  actions,
  themeIsDark,
  canAsk,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  recordings: RecordingRow[];
  onSelectRecording: (id: string) => void;
  capture: CaptureStatus;
  actions: PaletteActions;
  themeIsDark: boolean;
  canAsk: boolean;
}) {
  // Cmd+K / Ctrl+K from anywhere, including while a field has focus — the
  // whole point is that you never have to reach for the mouse first.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        onOpenChange(!open);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onOpenChange]);

  const now = useMemo(() => new Date(), []);
  const recording = capture.state === "recording" || capture.state === "paused";

  function run(fn: () => void) {
    onOpenChange(false);
    fn();
  }

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-black/35 backdrop-blur-[2px]" />
        <DialogPrimitive.Content
          aria-label="Command palette"
          className="fixed left-1/2 top-[15vh] z-50 w-[calc(100vw-2rem)] max-w-xl -translate-x-1/2 overflow-hidden rounded-[var(--radius-card)] border border-border bg-raised shadow-[var(--shadow-pop)]"
        >
          <DialogPrimitive.Title className="sr-only">Jump to anything</DialogPrimitive.Title>
          <Command loop className="flex flex-col">
            <Command.Input
              autoFocus
              placeholder="Search recordings, or type a command…"
              className="h-12 w-full border-b border-border bg-transparent px-4 text-[15px] text-fg placeholder:text-fg-faint focus:outline-none"
            />
            <Command.List className="max-h-[min(24rem,55vh)] overflow-y-auto p-1.5">
              <Command.Empty className="px-3 py-6 text-center text-[13px] text-fg-muted">
                Nothing matches. Full-text search across transcripts lives in the sidebar.
              </Command.Empty>

              <Command.Group
                heading="Do"
                className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-fg-faint"
              >
                {!recording && (
                  <>
                    <Row icon={<Circle size={14} />} onSelect={() => run(actions.startMeeting)}>
                      Record a meeting — both sides of the call
                    </Row>
                    <Row icon={<Mic size={14} />} onSelect={() => run(actions.startInPerson)}>
                      Record in person — microphone only
                    </Row>
                  </>
                )}
                {recording && (
                  <Row icon={<Square size={14} />} onSelect={() => run(actions.stop)}>
                    Stop recording
                  </Row>
                )}
                {canAsk && (
                  <Row icon={<Sparkles size={14} />} onSelect={() => run(actions.openAsk)}>
                    Ask about this recording
                  </Row>
                )}
                <Row
                  icon={themeIsDark ? <Sun size={14} /> : <Moon size={14} />}
                  onSelect={() => run(actions.toggleTheme)}
                >
                  Switch to {themeIsDark ? "light" : "dark"} mode
                </Row>
                <Row icon={<SettingsIcon size={14} />} onSelect={() => run(actions.openSettings)}>
                  Open settings
                </Row>
              </Command.Group>

              {recordings.length > 0 && (
                <Command.Group
                  heading="Recordings"
                  className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-fg-faint"
                >
                  {recordings.map((row) => (
                    <Row
                      key={row.id}
                      icon={<FileText size={14} />}
                      value={`${row.title} ${row.task ?? "Unsorted"}`}
                      onSelect={() => run(() => onSelectRecording(row.id))}
                      hint={`${dayLabel(row.created, now)} · ${roughDuration(row.durationS)}`}
                    >
                      {row.title}
                    </Row>
                  ))}
                </Command.Group>
              )}
            </Command.List>
          </Command>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function Row({
  icon,
  children,
  onSelect,
  value,
  hint,
}: {
  icon: React.ReactNode;
  children: React.ReactNode;
  onSelect: () => void;
  value?: string;
  hint?: string;
}) {
  return (
    <Command.Item
      value={value}
      onSelect={onSelect}
      className="flex cursor-pointer items-center gap-2.5 rounded-[var(--radius-control)] px-2 py-2 text-[14px] text-fg data-[selected=true]:bg-selected"
    >
      <span className="shrink-0 text-fg-faint">{icon}</span>
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {hint && <span className="shrink-0 text-[12px] text-fg-faint">{hint}</span>}
    </Command.Item>
  );
}
