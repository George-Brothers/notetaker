import { useCallback, useEffect, useState } from "react";

/**
 * "system" follows the OS; the other two override it. Stored as a plain string
 * so a value written by a future version with more themes degrades to "system"
 * rather than throwing.
 */
export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "notetaker.theme";

function readStored(): ThemePreference {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    return raw === "light" || raw === "dark" ? raw : "system";
  } catch {
    // Private browsing can refuse storage. Following the OS is the right
    // fallback — it is what the CSS does with no attribute set.
    return "system";
  }
}

/**
 * The light/dark preference, applied to the document.
 *
 * The CSS in `styles/theme.css` already handles `prefers-color-scheme`, so
 * "system" works by writing *no* attribute rather than by detecting the OS and
 * writing one. That matters: an explicitly-written "light" would not follow the
 * user changing their OS theme while the app is open, and this way the
 * following is the browser's job rather than a listener we could get wrong.
 */
export function useTheme() {
  const [preference, setPreference] = useState<ThemePreference>(readStored);

  useEffect(() => {
    const root = document.documentElement;
    if (preference === "system") {
      root.removeAttribute("data-theme");
    } else {
      root.setAttribute("data-theme", preference);
    }
    try {
      window.localStorage.setItem(STORAGE_KEY, preference);
    } catch {
      // Worst case the choice does not survive a restart.
    }
  }, [preference]);

  /**
   * What is actually on screen right now, which "system" alone does not tell
   * you — the toggle button needs it to show the right icon.
   */
  const [systemIsDark, setSystemIsDark] = useState(
    () => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false,
  );

  useEffect(() => {
    const query = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!query) return;
    const onChange = (e: MediaQueryListEvent) => setSystemIsDark(e.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  const resolved: "light" | "dark" =
    preference === "system" ? (systemIsDark ? "dark" : "light") : preference;

  /** Flips to the opposite of what is on screen, and stops following the OS. */
  const toggle = useCallback(() => {
    setPreference(resolved === "dark" ? "light" : "dark");
  }, [resolved]);

  return { preference, setPreference, resolved, toggle };
}
