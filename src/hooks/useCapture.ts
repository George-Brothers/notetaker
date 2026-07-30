import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/ipc";
import type { CaptureStatus, MeetingEvent, Mode } from "../lib/ipc";

const STATUS_POLL_MS = 1000;
const MEETING_POLL_MS = 3000;

const IDLE_STATUS: CaptureStatus = {
  state: "idle",
  mode: null,
  recordingId: null,
  elapsedS: 0,
  micLevel: 0,
  systemLevel: 0,
  diskFreeMb: Number.POSITIVE_INFINITY,
};

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function stamp(): string {
  return new Date().toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function meetingTitle(appName: string): string {
  return `${appName} meeting — ${stamp()}`;
}

/**
 * The name a recording gets when the caller supplies none — the command
 * palette, a keyboard shortcut, an auto-started meeting.
 *
 * Centralized here rather than at each call site so that "hit record and start
 * talking" can never produce an untitled recording. The pipeline suggests a
 * real title afterwards; this only has to be unambiguous in a list.
 */
function defaultTitle(mode: Mode): string {
  return `${mode === "meeting" ? "Meeting" : "In person"} ${stamp()}`;
}

/**
 * Owns the capture session (record bar) and the meeting-watcher prompt.
 * Plain React state, no library — matches the rest of this app.
 */
export function useCapture() {
  const [status, setStatus] = useState<CaptureStatus>(IDLE_STATUS);
  const [pendingMeeting, setPendingMeeting] = useState<MeetingEvent | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const statusTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.captureStatus());
    } catch (err) {
      setCaptureError(describeError(err));
    }
  }, []);

  // Hydrate once on mount (in case a capture is already running from a
  // previous session), then poll continuously only while a session is
  // active — never while idle.
  useEffect(() => {
    refreshStatus();
  }, [refreshStatus]);

  useEffect(() => {
    if (status.state === "idle") {
      return;
    }
    statusTimer.current = setInterval(refreshStatus, STATUS_POLL_MS);
    return () => {
      if (statusTimer.current) {
        clearInterval(statusTimer.current);
        statusTimer.current = null;
      }
    };
  }, [status.state, refreshStatus]);

  const start = useCallback(async (mode: Mode, title?: string) => {
    try {
      setStatus(await api.startCapture(mode, title?.trim() || defaultTitle(mode)));
      setCaptureError(null);
    } catch (err) {
      setCaptureError(describeError(err));
    }
  }, []);

  const pause = useCallback(async () => {
    try {
      setStatus(await api.pauseCapture());
    } catch (err) {
      setCaptureError(describeError(err));
    }
  }, []);

  const resume = useCallback(async () => {
    try {
      setStatus(await api.resumeCapture());
    } catch (err) {
      setCaptureError(describeError(err));
    }
  }, []);

  /**
   * Stops and returns the finished recording's id, or null if the stop failed.
   *
   * The id is returned rather than discarded so the caller can open the
   * recording that just ended — the notes typed while it ran should still be in
   * front of you afterwards, not somewhere in a list.
   */
  const stop = useCallback(async (): Promise<string | null> => {
    try {
      const id = await api.stopCapture();
      setStatus(IDLE_STATUS);
      return id;
    } catch (err) {
      setCaptureError(describeError(err));
      return null;
    }
  }, []);

  const pollMeetingEvents = useCallback(async () => {
    try {
      const events = await api.pollMeetings();
      for (const ev of events) {
        if (ev.kind !== "started") continue;
        if (ev.autoStart) {
          await start("meeting", meetingTitle(ev.appName));
        } else {
          setPendingMeeting(ev);
        }
      }
    } catch (err) {
      setCaptureError(describeError(err));
    }
  }, [start]);

  useEffect(() => {
    pollMeetingEvents();
    const id = setInterval(pollMeetingEvents, MEETING_POLL_MS);
    return () => clearInterval(id);
    // Intentionally runs for the component's whole lifetime: a meeting can
    // start at any time, not only while a capture is already in progress.
  }, [pollMeetingEvents]);

  const recordPendingMeeting = useCallback(async () => {
    if (!pendingMeeting) return;
    const ev = pendingMeeting;
    setPendingMeeting(null);
    await start("meeting", meetingTitle(ev.appName));
  }, [pendingMeeting, start]);

  const dismissPendingMeeting = useCallback(() => {
    setPendingMeeting(null);
  }, []);

  const alwaysRecordPending = useCallback(async () => {
    if (!pendingMeeting) return;
    const ev = pendingMeeting;
    setPendingMeeting(null);
    try {
      await api.setAutoRecord(ev.appId, "always");
    } catch (err) {
      setCaptureError(describeError(err));
    }
    await start("meeting", meetingTitle(ev.appName));
  }, [pendingMeeting, start]);

  const neverRecordPending = useCallback(async () => {
    if (!pendingMeeting) return;
    const ev = pendingMeeting;
    setPendingMeeting(null);
    try {
      await api.setAutoRecord(ev.appId, "never");
    } catch (err) {
      setCaptureError(describeError(err));
    }
  }, [pendingMeeting]);

  return {
    status,
    captureError,
    start,
    pause,
    resume,
    stop,
    pendingMeeting,
    recordPendingMeeting,
    dismissPendingMeeting,
    alwaysRecordPending,
    neverRecordPending,
  };
}
