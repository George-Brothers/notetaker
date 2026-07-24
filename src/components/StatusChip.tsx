import type { Status } from "../lib/ipc";

const LABELS: Record<Status, string> = {
  recorded: "Recorded",
  queued: "Queued",
  processing: "Processing",
  ready: "Ready",
  failed: "Failed",
};

const FALLBACK_ERROR = "Processing failed. Open the recording for details.";

export interface StatusChipProps {
  status: Status;
  /**
   * Plain-language failure reason. When status is "failed" this must be
   * shown to the user, not just attached as a hover tooltip — color alone
   * (or a title attribute alone) is not an acceptable way to communicate it.
   */
  error?: string | null;
}

export function StatusChip({ status, error }: StatusChipProps) {
  const trimmedError = error?.trim();
  return (
    <span className="status-chip-group">
      <span className={`status-chip status-chip--${status}`}>
        <span className="status-chip__dot" aria-hidden="true" />
        {LABELS[status]}
      </span>
      {status === "failed" && (
        <span className="status-chip__error">{trimmedError ? trimmedError : FALLBACK_ERROR}</span>
      )}
    </span>
  );
}
