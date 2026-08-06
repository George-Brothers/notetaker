import type { LiveTranscriptEvent } from "./ipc";

/**
 * Applies one chunked-batch transcript event without moving a speaker's live
 * line. The last non-final line for that speaker is the mutable slot; a final
 * event replaces that slot and leaves it frozen for subsequent turns.
 */
export function mergeLiveTranscript(
  messages: LiveTranscriptEvent[],
  event: LiveTranscriptEvent,
): LiveTranscriptEvent[] {
  let existing = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.speaker === event.speaker && message.isPartial && !message.isFinal) {
      existing = index;
      break;
    }
  }

  const next = [...messages];
  if (existing >= 0) {
    next[existing] = { ...event };
  } else {
    next.push({ ...event });
  }
  return next;
}
/** Keeps a transcript pane pinned only when the user is already at its end. */
export function isNearBottom(element: HTMLElement, threshold = 48): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight <= threshold;
}
