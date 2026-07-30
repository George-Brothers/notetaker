import type { Status } from "../lib/ipc";
import { cn } from "../lib/cn";

const LABELS: Record<Status, string> = {
  recorded: "Recorded",
  queued: "Queued",
  processing: "Processing",
  ready: "Ready",
  failed: "Failed",
};

const DOT: Record<Status, string> = {
  recorded: "bg-fg-faint",
  queued: "bg-fg-muted",
  processing: "bg-warn",
  ready: "bg-ok",
  failed: "bg-error",
};

const FALLBACK_ERROR = "Processing failed. Open the recording for details.";

export interface StatusChipProps {
  status: Status;
  /**
   * Plain-language failure reason. When status is "failed" this must be
   * shown to the user, not just attached as a hover tooltip — colour alone
   * (or a title attribute alone) is not an acceptable way to communicate it.
   */
  error?: string | null;
  /**
   * The sidebar variant: a dot and a word, no error text. The list row has no
   * room for a sentence, and the recording it belongs to shows the full
   * message the moment it is opened.
   */
  compact?: boolean;
}

export function StatusChip({ status, error, compact }: StatusChipProps) {
  const trimmedError = error?.trim();

  if (compact) {
    return (
      <span className="inline-flex items-center gap-1">
        <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", DOT[status])} />
        {LABELS[status]}
      </span>
    );
  }

  return (
    <span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
      <span className="inline-flex items-center gap-1.5 rounded-full border border-border bg-raised px-2 py-0.5 text-[12px] text-fg-muted">
        <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", DOT[status])} />
        {LABELS[status]}
      </span>
      {status === "failed" && (
        <span className="text-[12px] text-error">
          {trimmedError ? trimmedError : FALLBACK_ERROR}
        </span>
      )}
    </span>
  );
}
