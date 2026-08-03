import { useEffect, useMemo, useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Clock3,
  Loader2,
  Pause,
  Play,
  RotateCcw,
  Square,
  XCircle,
} from "lucide-react";
import type { QueueItem, QueueItemState, QueueSnapshot } from "../lib/ipc";
import { Button, IconButton } from "./ui";

function elapsed(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60);
  const secs = total % 60;
  return minutes > 0 ? `${minutes}m ${secs}s` : `${secs}s`;
}

const STATE_LABEL: Record<QueueItemState, string> = {
  queued: "Queued",
  processing: "Processing",
  completed: "Completed",
  retryable: "Retrying soon",
  failed: "Failed",
  paused: "Paused",
  cancelled: "Cancelled",
};

function StateIcon({ state }: { state: QueueItemState }) {
  if (state === "processing") return <Loader2 size={14} className="animate-spin text-accent" />;
  if (state === "completed") return <CheckCircle2 size={14} className="text-ok" />;
  if (state === "failed" || state === "retryable") return <CircleAlert size={14} className="text-error" />;
  if (state === "paused") return <Pause size={14} className="text-warn" />;
  if (state === "cancelled") return <XCircle size={14} className="text-fg-faint" />;
  return <Clock3 size={14} className="text-fg-faint" />;
}

function sortItems(items: QueueItem[]): QueueItem[] {
  const priority: Record<QueueItemState, number> = {
    processing: 0,
    queued: 1,
    retryable: 2,
    paused: 3,
    failed: 4,
    cancelled: 5,
    completed: 6,
  };
  return [...items].sort((a, b) => priority[a.state] - priority[b.state] || b.created.localeCompare(a.created));
}

export interface QueuePanelProps {
  snapshot: QueueSnapshot | null;
  error?: string | null;
  onSelectRecording: (id: string) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
}

export function QueuePanel({
  snapshot,
  error,
  onSelectRecording,
  onPause,
  onResume,
  onCancel,
  onRetry,
}: QueuePanelProps) {
  const [open, setOpen] = useState(false);
  const items = useMemo(() => sortItems(snapshot?.items ?? []), [snapshot?.items]);
  const active = items.filter((item) => item.state !== "completed");
  const hasRecentCompletion = items.some((item) => item.state === "completed");
  const current = active.find((item) => item.state === "processing") ?? active[0];

  useEffect(() => {
    if (current) setOpen(true);
  }, [current?.id]);

  const summary = !snapshot
    ? "Checking processing queue…"
    : current
      ? current.state === "processing"
        ? `${current.title} · ${current.stage ?? "Starting"}`
        : `${active.length} item${active.length === 1 ? "" : "s"} in the queue`
      : hasRecentCompletion
        ? "No processing pending · recent activity"
        : "Processing queue is empty";

  return (
    <section className="shrink-0 border-t border-border bg-raised" aria-label="Processing queue">
      <button
        type="button"
        className="flex min-h-10 w-full items-center gap-2 px-4 py-2 text-left text-[12px] hover:bg-hover"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {current ? <StateIcon state={current.state} /> : <CheckCircle2 size={14} className="text-ok" />}
        <span className="min-w-0 flex-1 truncate text-fg-muted">{summary}</span>
        {current?.state === "processing" && (
          <span className="shrink-0 text-fg-faint">{elapsed(current.elapsedS)}</span>
        )}
        <span className="shrink-0 text-fg-faint">
          {open ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
        </span>
      </button>

      {open && (
        <div className="max-h-64 overflow-y-auto border-t border-border px-4 py-2">
          {error && <p className="mb-2 text-[12px] text-error" role="alert">Queue status: {error}</p>}
          {snapshot && !snapshot.modelsReady && active.length > 0 && (
            <p className="mb-2 text-[12px] text-warn">
              Waiting for the speech models. Your recordings are safe and will stay in this queue.
            </p>
          )}
          {snapshot && !snapshot.processingEnabled && snapshot.modelsReady && active.length > 0 && (
            <p className="mb-2 text-[12px] text-fg-muted">
              Processing is not running. Restart Notetaker if this does not clear shortly.
            </p>
          )}
          {items.length === 0 ? (
            <p className="py-2 text-[12px] text-fg-faint">Nothing is waiting or recently completed.</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {items.slice(0, 12).map((item) => (
                <QueueRow
                  key={item.id}
                  item={item}
                  onSelect={() => onSelectRecording(item.id)}
                  onPause={() => onPause(item.id)}
                  onResume={() => onResume(item.id)}
                  onCancel={() => onCancel(item.id)}
                  onRetry={() => onRetry(item.id)}
                />
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}

function QueueRow({
  item,
  onSelect,
  onPause,
  onResume,
  onCancel,
  onRetry,
}: {
  item: QueueItem;
  onSelect: () => void;
  onPause: () => void;
  onResume: () => void;
  onCancel: () => void;
  onRetry: () => void;
}) {
  return (
    <li className="flex items-start gap-2 rounded-[var(--radius-control)] bg-sunken px-2.5 py-2">
      <span className="mt-0.5 shrink-0"><StateIcon state={item.state} /></span>
      <button type="button" onClick={onSelect} className="min-w-0 flex-1 text-left">
        <span className="block truncate text-[13px] font-medium text-fg">{item.title}</span>
        <span className="mt-0.5 block text-[11px] text-fg-muted">
          {STATE_LABEL[item.state]}
          {item.position != null && ` · #${item.position} in line`}
          {item.state === "processing" && item.stage && ` · ${item.stage}`}
          {item.state === "processing" && ` · ${elapsed(item.elapsedS)}`}
          {item.stageCount > 0 && item.state === "processing" && ` · Stage ${item.stageIndex} of ${item.stageCount}`}
        </span>
        {item.error && <span className="mt-0.5 block text-[11px] leading-snug text-error">{item.error}</span>}
      </button>
      <span className="flex shrink-0 items-center gap-0.5">
        {item.state === "queued" || item.state === "retryable" ? (
          <IconButton label="Pause processing" onClick={onPause}><Pause size={13} /></IconButton>
        ) : item.state === "paused" ? (
          <IconButton label="Resume processing" onClick={onResume}><Play size={13} /></IconButton>
        ) : null}
        {item.state === "queued" || item.state === "retryable" || item.state === "paused" ? (
          <IconButton label="Cancel processing" onClick={onCancel}><Square size={12} /></IconButton>
        ) : null}
        {item.state === "failed" || item.state === "cancelled" ? (
          <Button size="sm" variant="ghost" onClick={onRetry}><RotateCcw size={12} /> Retry</Button>
        ) : null}
      </span>
    </li>
  );
}
