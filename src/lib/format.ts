/**
 * Turning core's raw values into the strings a person reads.
 *
 * All of it is pure and tested, because these are the functions that quietly
 * produce "NaN:aN" or "Invalid Date" on a row nobody looked at.
 */

/**
 * A duration as `M:SS`, or `H:MM:SS` past an hour.
 *
 * Used for the elapsed timer and every recording's length, so it has to hold up
 * at 3 seconds and at 3 hours. Negative and non-finite inputs clamp to zero
 * rather than rendering `-1:-1` — a clock that reads 0:00 is wrong in a way a
 * user can ignore.
 */
export function duration(totalSeconds: number): string {
  const s = Number.isFinite(totalSeconds) ? Math.max(0, Math.floor(totalSeconds)) : 0;
  const hours = Math.floor(s / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;
  const mm = hours > 0 ? String(minutes).padStart(2, "0") : String(minutes);
  return hours > 0
    ? `${hours}:${mm}:${String(seconds).padStart(2, "0")}`
    : `${mm}:${String(seconds).padStart(2, "0")}`;
}

/**
 * A rough length for a list row: "45 min", "1 h 12 m", "under a minute".
 *
 * Deliberately coarser than `duration`. Scanning a library, the useful question
 * is "was this the long one", and `0:45:03` makes you do arithmetic to answer
 * it.
 */
export function roughDuration(totalSeconds: number): string {
  const s = Number.isFinite(totalSeconds) ? Math.max(0, Math.floor(totalSeconds)) : 0;
  if (s < 60) return "under a minute";
  const minutes = Math.round(s / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} m`;
}

/** Parses an RFC3339 string, returning null rather than an Invalid Date. */
function parse(rfc3339: string): Date | null {
  const d = new Date(rfc3339);
  return Number.isNaN(d.getTime()) ? null : d;
}

/**
 * The day heading a recording is filed under: "Today", "Yesterday", then a
 * date.
 *
 * `now` is a parameter so this is testable without freezing the clock, and so a
 * long-lived window cannot cache "Today" past midnight — callers pass a fresh
 * `new Date()`.
 */
export function dayLabel(rfc3339: string, now: Date = new Date()): string {
  const d = parse(rfc3339);
  if (!d) return "Undated";

  const midnight = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((midnight(now) - midnight(d)) / 86_400_000);

  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days > 1 && days < 7) return d.toLocaleDateString(undefined, { weekday: "long" });
  // A date in the future is a clock that disagrees with the file, not an error
  // worth surfacing — fall through to the plain date.
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString(
    undefined,
    sameYear
      ? { month: "long", day: "numeric" }
      : { year: "numeric", month: "long", day: "numeric" },
  );
}

/** The clock time a recording started: "2:30 PM". */
export function timeOfDay(rfc3339: string): string {
  const d = parse(rfc3339);
  if (!d) return "";
  return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

/** Full date and time, for the one place that shows a recording's identity. */
export function fullDateTime(rfc3339: string): string {
  const d = parse(rfc3339);
  if (!d) return "Unknown date";
  return d.toLocaleString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * A speaker's lane colour, as a CSS custom property.
 *
 * Assigned by position in the transcript's speaker list rather than by name, so
 * the colours stay put between visits. Wraps past five, which is more speakers
 * than a diarized recording reliably separates anyway.
 */
export function speakerColor(index: number): string {
  const lane = ((index % 5) + 5) % 5;
  return `var(--c-spk-${lane + 1})`;
}
