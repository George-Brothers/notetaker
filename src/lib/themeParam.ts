/**
 * `?theme=light|dark` pins the theme before first render by writing the same
 * localStorage key `useTheme` reads. Exists for deterministic screenshots
 * (scripts/shoot-ui.sh) and harmless for humans — the in-app toggle keeps
 * working because it writes the same key.
 */
export function applyThemeParam(
  search: string,
  storage: Pick<Storage, "setItem">,
): void {
  const value = new URLSearchParams(search).get("theme");
  if (value !== "light" && value !== "dark") return;
  try {
    storage.setItem("notetaker.theme", value);
  } catch {
    // Storage can refuse (private mode). Screenshots just fall back to OS.
  }
}
