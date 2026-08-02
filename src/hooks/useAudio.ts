/**
 * The one audio player in a note.
 *
 * This used to live inside `TranscriptPanel`, below its no-segments early
 * return — which is why a recording that had not been processed yet had no
 * way to reach it at all. It lives here so `NoteView` can own a single
 * player and hand the same one to the transcript.
 *
 * `seek` deliberately does not start playback: skipping ten seconds or
 * dragging the scrubber while paused should leave you paused. Only
 * `seekAndPlay` starts, and only the transcript's click-a-line uses it.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { audioSrc } from "../lib/transport";

/** How far the skip buttons move. */
export const SKIP_SECONDS = 10;

/** Playback speeds, slowest first. 0.75 earns its place on bilingual audio. */
export const RATES: readonly number[] = [0.75, 1, 1.25, 1.5, 1.75, 2];

export const RATE_KEY = "notetaker.playbackRate";

/**
 * Where a seek actually lands.
 *
 * Pure on purpose — jsdom's media element does not play, so this is the only
 * part of seeking that can be tested directly, and it is the part with the
 * edges. A `durationS` of 0 means "not known yet" and imposes no ceiling.
 */
export function clampSeek(seconds: number, durationS: number): number {
  if (!Number.isFinite(seconds) || seconds < 0) return 0;
  if (durationS > 0 && seconds > durationS) return durationS;
  return seconds;
}

/**
 * The saved speed, or normal speed. Pure, so the "someone hand-edited
 * localStorage" cases are testable without touching storage.
 */
export function readRate(raw: string | null): number {
  const n = raw === null ? Number.NaN : Number(raw);
  return RATES.includes(n) ? n : 1;
}

function storedRate(): number {
  try {
    return readRate(window.localStorage.getItem(RATE_KEY));
  } catch {
    // Storage can throw in private mode. A speed preference is not worth
    // taking the player down with it.
    return 1;
  }
}

export interface AudioPlayer {
  ref: React.MutableRefObject<HTMLAudioElement | null>;
  src: string | null;
  error: string | null;
  playing: boolean;
  setPlaying: (playing: boolean) => void;
  currentTime: number;
  setCurrentTime: (seconds: number) => void;
  seek: (seconds: number) => void;
  seekAndPlay: (seconds: number) => void;
  skip: (delta: number) => void;
  toggle: () => void;
  rate: number;
  setRate: (rate: number) => void;
}

export function useAudio(
  recordingId: string,
  track: string | null,
  durationS: number,
): AudioPlayer {
  const ref = useRef<HTMLAudioElement | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [rate, setRateState] = useState(storedRate);

  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    setError(null);
    setCurrentTime(0);
    setPlaying(false);
    if (!track) return;
    audioSrc(recordingId, track)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [recordingId, track]);

  // Re-applied on every src change too: a fresh element starts at 1×.
  useEffect(() => {
    const el = ref.current;
    if (el) el.playbackRate = rate;
  }, [rate, src]);

  const setRate = useCallback((next: number) => {
    setRateState(next);
    try {
      window.localStorage.setItem(RATE_KEY, String(next));
    } catch {
      // See storedRate.
    }
  }, []);

  const seek = useCallback(
    (seconds: number) => {
      const el = ref.current;
      if (!el) return;
      const landed = clampSeek(seconds, durationS);
      el.currentTime = landed;
      setCurrentTime(landed);
    },
    [durationS],
  );

  const seekAndPlay = useCallback(
    (seconds: number) => {
      seek(seconds);
      const el = ref.current;
      if (!el) return;
      void Promise.resolve(el.play()).catch(() => {
        // Autoplay policy can refuse a play() the user did not initiate.
        // The seek still landed; they can press play.
      });
    },
    [seek],
  );

  const skip = useCallback(
    (delta: number) => {
      const el = ref.current;
      if (!el) return;
      seek(el.currentTime + delta);
    },
    [seek],
  );

  const toggle = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    if (el.paused) void Promise.resolve(el.play()).catch(() => setPlaying(false));
    else el.pause();
  }, []);

  return {
    ref,
    src,
    error,
    playing,
    setPlaying,
    currentTime,
    setCurrentTime,
    seek,
    seekAndPlay,
    skip,
    toggle,
    rate,
    setRate,
  };
}
