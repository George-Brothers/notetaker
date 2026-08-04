# UI/UX Overhaul Implementation Plan — "Lit from within"

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute the approved 2026-08-04 overhaul: aurora identity in two
materials, Echo mascot + icon pipeline, rebuilt Settings with hotkeys, library
sort/filter, find-and-jump palette, and the native layer (tray, global
hotkeys, autostart, single instance, custom titlebar).

**Architecture:** Layered overhaul on the existing tested components. Phase A
(Tasks 1–7) is pure frontend + core-crate Rust and fully verifiable on this
WSL2 box. Phase B (Tasks 8–10) is the Tauri app-crate native layer, compiled
and verified by CI/Windows because the app crate cannot build here. Task 11
is the visual polish pass; Task 12 is the sweep.

**Tech Stack:** React 19 + TS + Tailwind 4 tokens, Radix, cmdk, lucide;
Tauri 2 (plugins: global-shortcut, autostart, single-instance, window-state,
dialog, + existing updater/opener/process); Rust core crate for settings.

**Spec:** `docs/superpowers/specs/2026-08-04-ui-overhaul-design.md` — every
color, string, and default comes from there. **If a value is not in the spec
or this plan, do not invent it: stop and report.**

## Global Constraints

- **Worktree:** all work happens in
  `/home/georg/projects/notetaker personal/.claude/worktrees/app-ui-ux-overhaul-96e4c6`
  on branch `claude/app-ui-ux-overhaul-96e4c6` (already based on `561ecb0`).
- **Frontend commands** run at the repo root: `pnpm install` (once),
  `pnpm test --run`, `pnpm build`. `pnpm build` is the only typecheck — run it
  even when tests pass.
- **Rust commands** run from `src-tauri/`:
  `PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core`
  and the same env for `cargo clippy -p notetaker-core --all-targets -- -D warnings`.
- **The app crate (`src-tauri/src/`) does NOT compile on this machine** (no
  sudo for webkit/dbus). Tasks 8–10 verify Rust by pushing the branch and
  watching CI (`gh run list --branch claude/app-ui-ux-overhaul-96e4c6`,
  `gh run watch <id>`). If the pre-push guard blocks the push, STOP and
  surface it to Mr. Brothers — never bypass a guard, never `--no-verify`.
- **Do not edit** `src-tauri/core/src/capture/**`, `queue/**`, `storage/**`,
  `index/**`, or anything under `src-tauri/server/` except where a task names
  the file explicitly.
- **Never rename or remove an existing CSS token; additive only** per spec §2.
- **Test-visible copy is law:** control labels asserted in
  `src/components/__tests__/settings.test.tsx` (e.g. "Where recordings are
  saved", "Wait until I'm not using the computer", "Only process while
  plugged in", "Keep the original recording file too", "Speech model",
  "Check for updates", "Open the log folder") must survive the Settings
  rebuild verbatim. New copy comes only from the spec.
- **Screenshots** use the cached Chromium:
  `CHROME=$(ls -d ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome | head -1)`.
- **A failing check fixed twice and still failing → stop and report.** Do not
  loop silently.
- Commits: small, per task, message style matches `git log` (lower-case
  conventional prefixes), each ending with:
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`
- Do not merge, do not open a PR, do not deploy — Mr. Brothers' word gates
  those. Pushing this branch for CI verification is allowed if the guard
  permits it.

---

### Task 1: The token foundation — theme.css, motion, deterministic theme for screenshots

**Files:**
- Modify: `src/styles/theme.css` (replace the value blocks; keep structure)
- Create: `src/lib/themeParam.ts`
- Create: `src/lib/__tests__/themeParam.test.ts`
- Modify: `src/main.tsx` (3 lines)
- Modify: `index.html` (2 meta values)
- Create: `scripts/shoot-ui.sh`

**Interfaces:**
- Consumes: nothing.
- Produces: every token in spec §2 (existing names + new `--c-accent-2`,
  `--c-accent-2-soft`, `--grad-aurora`, `--glow-accent`, `--glow-recording`,
  `--t-fast`, `--t-med`, `--t-slow`, `--ease-swift`); Tailwind utilities
  `bg-accent-2`, `text-accent-2`, `bg-accent-2-soft`;
  `applyThemeParam(search: string, storage: Pick<Storage,"setItem">): void`;
  `scripts/shoot-ui.sh <outdir>` producing `light.png` and `dark.png`.

- [ ] **Step 1: Install deps and confirm a green baseline**

```bash
cd "/home/georg/projects/notetaker personal/.claude/worktrees/app-ui-ux-overhaul-96e4c6"
pnpm install
pnpm test --run && pnpm build
```
Expected: all suites pass, build clean. If not, STOP — the baseline is broken.

- [ ] **Step 2: Write the failing test for the theme URL param**

Create `src/lib/__tests__/themeParam.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { applyThemeParam } from "../themeParam";

describe("applyThemeParam", () => {
  it("stores a valid ?theme= value under the useTheme key", () => {
    const setItem = vi.fn();
    applyThemeParam("?theme=dark", { setItem });
    expect(setItem).toHaveBeenCalledWith("notetaker.theme", "dark");
  });

  it("ignores absent and invalid values", () => {
    const setItem = vi.fn();
    applyThemeParam("", { setItem });
    applyThemeParam("?theme=neon", { setItem });
    expect(setItem).not.toHaveBeenCalled();
  });

  it("swallows storage errors", () => {
    const setItem = vi.fn(() => {
      throw new Error("private mode");
    });
    expect(() => applyThemeParam("?theme=light", { setItem })).not.toThrow();
  });
});
```

- [ ] **Step 3: Run it, verify it fails**

Run: `pnpm test --run -- themeParam`
Expected: FAIL — `Cannot find module '../themeParam'`.

- [ ] **Step 4: Implement `src/lib/themeParam.ts`**

```ts
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
```

And in `src/main.tsx`, add after the imports (before `createRoot`):

```ts
import { applyThemeParam } from "./lib/themeParam";

applyThemeParam(window.location.search, window.localStorage);
```

- [ ] **Step 5: Tests pass**

Run: `pnpm test --run -- themeParam`  → PASS.

- [ ] **Step 6: Replace the palette values in `src/styles/theme.css`**

Keep the file's structure, comments, and mechanism exactly (two-layer theme,
`@theme inline` bridge, reduced-motion block). Make precisely these changes:

**(a)** In `@theme` (top block), after `--radius-control: 0.5rem;` add:

```css
  /* Motion: one speed system. `--ease-swift` is the app's only easing. */
  --t-fast: 120ms;
  --t-med: 200ms;
  --t-slow: 280ms;
  --ease-swift: cubic-bezier(0.2, 0, 0, 1);
```

**(b)** Replace the `:root { … }` light values (first block) with:

```css
    --c-app: #f7f7fa;
    --c-raised: #ffffff;
    --c-sunken: #efeff5;
    --c-hover: #e9e9f1;
    --c-selected: #e7e4fa;

    --c-border: #e3e3ec;
    --c-border-strong: #cfcfdd;

    --c-fg: #17171f;
    --c-fg-ai: #5d5b75;
    --c-fg-muted: #79778e;
    --c-fg-faint: #a3a1b8;

    --c-accent: #6c4ff0;
    --c-accent-hover: #5b3fe0;
    --c-accent-fg: #ffffff;
    --c-accent-soft: #eeeafe;
    --c-accent-2: #0e7fa8;
    --c-accent-2-soft: #e3f4fb;

    --c-recording: #e0342b;
    --c-recording-soft: #fce9e7;
    --c-warn: #b07514;
    --c-warn-soft: #fbf1dd;
    --c-error: #c92f26;
    --c-error-soft: #fbe9e7;
    --c-ok: #1f9d6c;
    --c-ok-soft: #e3f5ee;

    --c-spk-1: #0e7fa8;
    --c-spk-2: #7c4fd8;
    --c-spk-3: #b26a1b;
    --c-spk-4: #1e8f68;
    --c-spk-5: #c24a7e;

    --grad-aurora: linear-gradient(92deg, var(--c-accent), var(--c-accent-2));
    --glow-accent: 0 3px 14px rgb(108 79 240 / 0.2);
    --glow-recording: 0 2px 12px rgb(224 52 43 / 0.25);

    --shadow-card: 0 1px 2px rgb(23 23 31 / 0.05),
      0 4px 14px rgb(108 79 240 / 0.06);
    --shadow-pop: 0 6px 18px rgb(23 23 31 / 0.1),
      0 20px 48px rgb(108 79 240 / 0.14);
```

**(c)** Replace the dark values inside `@media (prefers-color-scheme: dark)` with:

```css
      --c-app: #0b0b12;
      --c-raised: #14141f;
      --c-sunken: #07070c;
      --c-hover: #1c1c2b;
      --c-selected: #232338;

      --c-border: #232336;
      --c-border-strong: #34344e;

      --c-fg: #ededf7;
      --c-fg-ai: #a9a9c5;
      --c-fg-muted: #8a8aa8;
      --c-fg-faint: #62627e;

      --c-accent: #8b72ff;
      --c-accent-hover: #9d87ff;
      --c-accent-fg: #0b0b12;
      --c-accent-soft: #241f45;
      --c-accent-2: #4dd6ff;
      --c-accent-2-soft: #10333f;

      --c-recording: #ff5c51;
      --c-recording-soft: #3a1815;
      --c-warn: #f5b84d;
      --c-warn-soft: #2e2412;
      --c-error: #ff6b61;
      --c-error-soft: #3a1815;
      --c-ok: #3ed9a4;
      --c-ok-soft: #10352a;

      --c-spk-1: #4dd6ff;
      --c-spk-2: #b78cff;
      --c-spk-3: #ffb86b;
      --c-spk-4: #63e6be;
      --c-spk-5: #ff8fb8;

      --grad-aurora: linear-gradient(92deg, var(--c-accent), var(--c-accent-2));
      --glow-accent: 0 0 20px rgb(139 114 255 / 0.35);
      --glow-recording: 0 0 16px rgb(255 92 81 / 0.45);

      --shadow-card: 0 1px 2px rgb(0 0 0 / 0.5), 0 4px 16px rgb(0 0 0 / 0.35);
      --shadow-pop: 0 8px 24px rgb(0 0 0 / 0.55), 0 24px 64px rgb(0 0 0 / 0.6);
```

**(d)** Replace `:root[data-theme="light"]` with the full light list from (b)
and `:root[data-theme="dark"]` with the full dark list from (c) — both blocks
repeated **in full**, exactly like the file does today (the comment above them
explains why).

**(e)** In the `@theme inline` bridge, after the `--color-selected` line add:

```css
  --color-accent-2: var(--c-accent-2);
  --color-accent-2-soft: var(--c-accent-2-soft);
```

**(f)** In `index.html`, update the two `theme-color` metas:
light `content="#faf8f3"` → `content="#f7f7fa"`;
dark `content="#16150f"` → `content="#0b0b12"`.

- [ ] **Step 7: Machine checks**

```bash
pnpm test --run && pnpm build
grep -c "grad-aurora" src/styles/theme.css   # expected: 4 (2 defs + 2 override blocks)
grep -c "#2f6f4e\|#faf8f3\|#16150f" src/styles/theme.css   # expected: 0 — old palette gone
```
Expected: tests green (they assert behavior, not colors), build clean, greps as stated.

- [ ] **Step 8: Create `scripts/shoot-ui.sh` (mode 0755)**

```bash
#!/usr/bin/env bash
# Screenshots the real UI (built frontend + notetaker-serve backend) in both
# themes. Usage: scripts/shoot-ui.sh <outdir>   → <outdir>/{light,dark}.png
set -euo pipefail
OUT="${1:?usage: shoot-ui.sh <outdir>}"; mkdir -p "$OUT"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHROME=$(ls -d ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome | head -1)

cd "$ROOT" && pnpm build
cd "$ROOT/src-tauri"
export PATH="$HOME/.cargo/bin:$PATH" LIBCLANG_PATH="$HOME/.local/lib/libclang"
cargo build -p notetaker-server --bin serve
export LD_LIBRARY_PATH="$ROOT/src-tauri/target/debug"
LOG=$(mktemp)
NOTETAKER_ROOT_OVERRIDE="" ./target/debug/serve --port 14211 --ui-dir "$ROOT/dist" >"$LOG" 2>&1 &
SERVE_PID=$!
trap 'kill $SERVE_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  URL=$(grep -oE 'http://[0-9.]+:14211[^ ]*' "$LOG" | head -1 || true)
  [ -n "$URL" ] && curl -sf "$URL" >/dev/null 2>&1 && break
  sleep 0.5
done
[ -n "${URL:-}" ] || { echo "serve never printed its URL"; cat "$LOG"; exit 1; }
SEP='?'; case "$URL" in *\?*) SEP='&';; esac
"$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
  --window-size=1280,800 --screenshot="$OUT/light.png" "${URL}${SEP}theme=light"
"$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
  --window-size=1280,800 --screenshot="$OUT/dark.png" "${URL}${SEP}theme=dark"
echo "wrote $OUT/light.png and $OUT/dark.png"
```

Note: if `serve --help` shows different flag names than `--port`/`--ui-dir`,
match the script to `--help` output and say so in the commit body. If the
printed URL carries a `?token=`, the `SEP` logic already keeps it.

- [ ] **Step 9: Shoot and compare**

Run: `bash scripts/shoot-ui.sh /tmp/overhaul-shots/task1`
Expected: two PNGs. Open both. Acceptance beats: dark background is near-black
violet (`#0B0B12`), light is porcelain (`#F7F7FA`), accent on interactive
elements is violet, recording chip red, no green accents anywhere. Compare
against `docs/superpowers/specs/assets/2026-08-04-pitch/pitch-top.png` Plate 02.

- [ ] **Step 10: Commit**

```bash
git add src/styles/theme.css src/lib/themeParam.ts src/lib/__tests__/themeParam.test.ts src/main.tsx index.html scripts/shoot-ui.sh
git commit -m "feat(ui): the aurora token system — luminous glass and porcelain

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Library sort and filter

**Files:**
- Modify: `src/hooks/useLibrary.ts`
- Modify: `src/components/Sidebar.tsx`
- Test: `src/components/__tests__/sidebar.test.tsx` (add cases)

**Interfaces:**
- Consumes: `RecordingRow` from `src/lib/ipc.ts`.
- Produces: `export type SortKey = "newest" | "oldest" | "longest" | "alpha"`;
  `export type FilterKey = "all" | "processing" | "error" | "notes"` (both from
  `useLibrary.ts`); `useLibrary()` additionally returns
  `{ sort, setSort, filter, setFilter }`; `Sidebar` gains required props
  `sort: SortKey; onSetSort: (s: SortKey) => void; filter: FilterKey; onSetFilter: (f: FilterKey) => void`.
  localStorage keys: `notetaker.librarySort`, `notetaker.libraryFilter`.

- [ ] **Step 1: Write the failing tests** — append to
`src/components/__tests__/sidebar.test.tsx` (match the file's existing render
helper; pass the new required props with defaults `sort="newest"`,
`filter="all"`, `onSetSort`/`onSetFilter` as `vi.fn()` in the existing helper
so old cases keep compiling):

```tsx
describe("sort and filter controls", () => {
  it("renders the sort control showing the active order", () => {
    renderSidebar({ sort: "newest" });
    expect(screen.getByLabelText("Sort recordings")).toHaveValue("newest");
  });

  it("changing sort calls onSetSort", () => {
    const onSetSort = vi.fn();
    renderSidebar({ onSetSort });
    fireEvent.change(screen.getByLabelText("Sort recordings"), {
      target: { value: "longest" },
    });
    expect(onSetSort).toHaveBeenCalledWith("longest");
  });

  it("changing filter calls onSetFilter", () => {
    const onSetFilter = vi.fn();
    renderSidebar({ onSetFilter });
    fireEvent.change(screen.getByLabelText("Show only"), {
      target: { value: "error" },
    });
    expect(onSetFilter).toHaveBeenCalledWith("error");
  });

  it("hides day headers when sorted by length", () => {
    renderSidebar({ sort: "longest", recordings: [ROW_TODAY, ROW_YESTERDAY] });
    expect(screen.queryByText("Today")).not.toBeInTheDocument();
  });
});
```

(`ROW_TODAY`/`ROW_YESTERDAY`: reuse the file's existing row fixtures; if none
are day-distinct, build two rows whose `created` differ by one day.)

Add to `src/hooks/__tests__/` a new file `useLibrarySort.test.ts` testing the
pure helpers (exported for tests):

```ts
import { describe, expect, it } from "vitest";
import { applySort, applyFilter } from "../useLibrary";
import type { RecordingRow } from "../../lib/ipc";

const row = (over: Partial<RecordingRow>): RecordingRow => ({
  id: "r1", title: "A", task: null, created: "2026-08-04T10:00:00Z",
  durationS: 60, mode: "meeting", status: "ready", suggestedTask: null,
  suggestedTitle: null, hasNotes: false, error: null, captureNote: null,
  ...over,
});

describe("applySort", () => {
  const rows = [
    row({ id: "old", created: "2026-08-01T10:00:00Z", durationS: 300, title: "Beta" }),
    row({ id: "new", created: "2026-08-04T10:00:00Z", durationS: 60, title: "alpha" }),
  ];
  it("newest first by default", () => {
    expect(applySort(rows, "newest").map((r) => r.id)).toEqual(["new", "old"]);
  });
  it("oldest", () => {
    expect(applySort(rows, "oldest").map((r) => r.id)).toEqual(["old", "new"]);
  });
  it("longest", () => {
    expect(applySort(rows, "longest").map((r) => r.id)).toEqual(["old", "new"]);
  });
  it("alpha is case-insensitive", () => {
    expect(applySort(rows, "alpha").map((r) => r.title)).toEqual(["alpha", "Beta"]);
  });
});

describe("applyFilter", () => {
  const rows = [
    row({ id: "p", status: "processing" }),
    row({ id: "q", status: "queued" }),
    row({ id: "f", status: "failed", error: "boom" }),
    row({ id: "n", hasNotes: true }),
    row({ id: "r" }),
  ];
  it("all passes everything", () => {
    expect(applyFilter(rows, "all")).toHaveLength(5);
  });
  it("processing means queued or processing", () => {
    expect(applyFilter(rows, "processing").map((r) => r.id)).toEqual(["p", "q"]);
  });
  it("error means failed", () => {
    expect(applyFilter(rows, "error").map((r) => r.id)).toEqual(["f"]);
  });
  it("notes means hasNotes", () => {
    expect(applyFilter(rows, "notes").map((r) => r.id)).toEqual(["n"]);
  });
});
```

- [ ] **Step 2: Run to verify failure** —
`pnpm test --run -- sidebar useLibrarySort` → FAIL (missing exports/props).

- [ ] **Step 3: Implement `useLibrary.ts`** — add below `filterByView`:

```ts
export type SortKey = "newest" | "oldest" | "longest" | "alpha";
export type FilterKey = "all" | "processing" | "error" | "notes";

const SORT_STORAGE_KEY = "notetaker.librarySort";
const FILTER_STORAGE_KEY = "notetaker.libraryFilter";

const SORT_KEYS: readonly SortKey[] = ["newest", "oldest", "longest", "alpha"];
const FILTER_KEYS: readonly FilterKey[] = ["all", "processing", "error", "notes"];

function readStored<T extends string>(key: string, valid: readonly T[], fallback: T): T {
  try {
    const raw = window.localStorage.getItem(key);
    return valid.includes(raw as T) ? (raw as T) : fallback;
  } catch {
    return fallback;
  }
}

/** Pure so the ordering rules are unit-testable. Exported for tests. */
export function applySort(rows: RecordingRow[], sort: SortKey): RecordingRow[] {
  const copy = [...rows];
  switch (sort) {
    case "newest":
      return copy.sort((a, b) => b.created.localeCompare(a.created));
    case "oldest":
      return copy.sort((a, b) => a.created.localeCompare(b.created));
    case "longest":
      return copy.sort((a, b) => b.durationS - a.durationS);
    case "alpha":
      return copy.sort((a, b) =>
        a.title.localeCompare(b.title, undefined, { sensitivity: "base" }),
      );
  }
}

/** Pure so the visibility rules are unit-testable. Exported for tests. */
export function applyFilter(rows: RecordingRow[], filter: FilterKey): RecordingRow[] {
  switch (filter) {
    case "all":
      return rows;
    case "processing":
      return rows.filter((r) => r.status === "queued" || r.status === "processing");
    case "error":
      return rows.filter((r) => r.status === "failed");
    case "notes":
      return rows.filter((r) => r.hasNotes);
  }
}
```

Inside `useLibrary()` add state + persistence (below the `view` state):

```ts
  const [sort, setSortState] = useState<SortKey>(() =>
    readStored(SORT_STORAGE_KEY, SORT_KEYS, "newest"),
  );
  const [filter, setFilterState] = useState<FilterKey>(() =>
    readStored(FILTER_STORAGE_KEY, FILTER_KEYS, "all"),
  );
  const setSort = useCallback((s: SortKey) => {
    setSortState(s);
    try { window.localStorage.setItem(SORT_STORAGE_KEY, s); } catch { /* best effort */ }
  }, []);
  const setFilter = useCallback((f: FilterKey) => {
    setFilterState(f);
    try { window.localStorage.setItem(FILTER_STORAGE_KEY, f); } catch { /* best effort */ }
  }, []);
```

Change `filterByView` to stop pre-sorting (it currently calls
`sortByCreatedDesc`): replace its body's `const sorted = sortByCreatedDesc(rows);`
with `const sorted = rows;` (the memo below owns order now; keep the function
name), and replace the `visibleRecordings` memo with:

```ts
  const visibleRecordings = useMemo(
    () => applySort(applyFilter(filterByView(source, view), filter), sort),
    [source, view, filter, sort],
  );
```

Delete the now-unused `sortByCreatedDesc` (the "recent" case comment about
newest-first stays true via the default sort). Return `sort, setSort, filter,
setFilter` from the hook.

- [ ] **Step 4: Implement the Sidebar UI** — add the four props to
`SidebarProps` (required, typed from `useLibrary` exports). Under the
palette-hint button (after line ~226, inside the top `div`), insert:

```tsx
        <div className="flex items-center gap-1.5">
          <label htmlFor="library-sort" className="sr-only">Sort recordings</label>
          <select
            id="library-sort"
            aria-label="Sort recordings"
            value={sort}
            onChange={(e) => onSetSort(e.target.value as SortKey)}
            className="h-6 flex-1 cursor-pointer rounded-[var(--radius-control)] border border-border bg-raised px-1.5 text-[11px] font-medium text-fg-muted focus:border-accent focus:outline-none"
          >
            <option value="newest">Newest first</option>
            <option value="oldest">Oldest first</option>
            <option value="longest">Longest first</option>
            <option value="alpha">A to Z</option>
          </select>
          <label htmlFor="library-filter" className="sr-only">Show only</label>
          <select
            id="library-filter"
            aria-label="Show only"
            value={filter}
            onChange={(e) => onSetFilter(e.target.value as FilterKey)}
            className="h-6 flex-1 cursor-pointer rounded-[var(--radius-control)] border border-border bg-raised px-1.5 text-[11px] font-medium text-fg-muted focus:border-accent focus:outline-none"
          >
            <option value="all">Everything</option>
            <option value="processing">Still processing</option>
            <option value="error">Had a problem</option>
            <option value="notes">Has my notes</option>
          </select>
        </div>
```

The selected row gains the accent edge from spec §3 — in `RecordingItem`, the
selected class string `"bg-selected"` becomes
`"bg-selected shadow-[inset_2px_0_0_var(--c-accent)]"` (the non-selected
branch is untouched).

Day headers only under date sorts — where `groups.map(...)` renders, branch:

```tsx
              {sort === "newest" || sort === "oldest" ? (
                groups.map((group) => ( /* existing grouped rendering, unchanged */ ))
              ) : (
                <ul className="flex flex-col gap-0.5">
                  {recordings.map((row) => (
                    <li key={row.id}>
                      <RecordingItem
                        row={row}
                        selected={row.id === selectedId}
                        onSelect={() => onSelectRecording(row.id)}
                        modelsMissing={modelsMissing}
                      />
                    </li>
                  ))}
                </ul>
              )}
```

(`sort` comes from props; import the types:
`import type { FilterKey, LibraryView, SortKey } from "../hooks/useLibrary";`.)

In `src/App.tsx`, pass the new props where `<Sidebar` renders:
`sort={lib.sort} onSetSort={lib.setSort} filter={lib.filter} onSetFilter={lib.setFilter}`.

- [ ] **Step 5: All green** — `pnpm test --run && pnpm build` → PASS. If an
existing library test asserted implicit newest-first, it must still pass —
default sort is `newest`.

- [ ] **Step 6: Commit**

```bash
git add src/hooks/useLibrary.ts src/components/Sidebar.tsx src/App.tsx src/components/__tests__/sidebar.test.tsx src/hooks/__tests__/useLibrarySort.test.ts
git commit -m "feat(ui): sort and filter the library — newest, oldest, longest, A to Z

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Settings fields for devices, hotkeys, and tray (core Rust + ipc.ts)

**Files:**
- Modify: `src-tauri/core/src/api.rs` (Settings struct + defaults + tests)
- Modify: `src/lib/ipc.ts` (Settings interface)

**Interfaces:**
- Produces (Rust, `#[serde(rename_all = "camelCase")]` already on the struct):
  `input_device: Option<String>`, `hotkey_toggle_record: String`
  (default `"CommandOrControl+Alt+N"`), `hotkey_show_hide: String`
  (default `"CommandOrControl+Alt+Space"`), `close_to_tray: bool` (default true).
- Produces (TS): `Settings` gains `inputDevice: string | null;
  hotkeyToggleRecord: string; hotkeyShowHide: string; closeToTray: boolean;`.

- [ ] **Step 1: Failing Rust test** — at the bottom of
`src-tauri/core/src/api.rs` add:

```rust
#[cfg(test)]
mod overhaul_settings_tests {
    use super::*;

    /// A settings file written before the overhaul must parse, landing on the
    /// documented defaults instead of resetting the user's config.
    #[test]
    fn pre_overhaul_settings_json_gets_defaults() {
        let old = r#"{
            "storageRoot": "/tmp/x",
            "llmBaseUrl": "http://localhost:11434",
            "llmModel": "qwen3:8b",
            "tierOverride": null,
            "processWhenIdle": true
        }"#;
        let s: Settings = serde_json::from_str(old).expect("old settings must parse");
        assert_eq!(s.input_device, None);
        assert_eq!(s.hotkey_toggle_record, "CommandOrControl+Alt+N");
        assert_eq!(s.hotkey_show_hide, "CommandOrControl+Alt+Space");
        assert!(s.close_to_tray);
    }

    /// Round-trip: the new fields serialize camelCase, matching ipc.ts.
    #[test]
    fn new_fields_serialize_camel_case() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains("\"inputDevice\":null"));
        assert!(json.contains("\"hotkeyToggleRecord\":\"CommandOrControl+Alt+N\""));
        assert!(json.contains("\"hotkeyShowHide\":\"CommandOrControl+Alt+Space\""));
        assert!(json.contains("\"closeToTray\":true"));
    }
}
```

- [ ] **Step 2: Verify failure**

Run (from `src-tauri/`):
`PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core overhaul_settings`
Expected: compile error — fields missing.

- [ ] **Step 3: Add the fields** — in `pub struct Settings`, after
`pub speech_engine: SpeechEngine,` append:

```rust
    // --- 2026-08-04 UI overhaul. All defaulted so any older settings file
    // parses unchanged; see docs/superpowers/specs/2026-08-04-ui-overhaul-design.md §7.
    /// Which input device records. `None` means the system default.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Global accelerator that starts/stops recording, Tauri notation.
    #[serde(default = "default_hotkey_toggle_record")]
    pub hotkey_toggle_record: String,
    /// Global accelerator that shows/hides the window, Tauri notation.
    #[serde(default = "default_hotkey_show_hide")]
    pub hotkey_show_hide: String,
    /// Closing the window hides to the tray instead of quitting.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
```

Next to the other default fns add:

```rust
fn default_hotkey_toggle_record() -> String {
    "CommandOrControl+Alt+N".to_string()
}

fn default_hotkey_show_hide() -> String {
    "CommandOrControl+Alt+Space".to_string()
}
```

Extend `impl Default for Settings` with:

```rust
            input_device: None,
            hotkey_toggle_record: default_hotkey_toggle_record(),
            hotkey_show_hide: default_hotkey_show_hide(),
            close_to_tray: true,
```

- [ ] **Step 4: Green + clippy** — same env:
`cargo test -p notetaker-core` (full suite; other tests construct
`Settings { .. }` via `Default` or serde, so additive-with-default is safe —
if any test constructs the struct literally, add the four fields there too)
and `cargo clippy -p notetaker-core --all-targets -- -D warnings` → both clean.

- [ ] **Step 5: Mirror in `src/lib/ipc.ts`** — inside `interface Settings`,
after `speechEngine: SpeechEngine;` add:

```ts
  /** Which input device records. null means the system default. */
  inputDevice: string | null;
  /** Global start/stop-recording accelerator, Tauri notation. */
  hotkeyToggleRecord: string;
  /** Global show/hide-window accelerator, Tauri notation. */
  hotkeyShowHide: string;
  /** Closing the window hides to the tray instead of quitting. */
  closeToTray: boolean;
```

Then fix every test fixture that builds a full `Settings` object
(`grep -rn "speechEngine" src/ --include="*.tsx" --include="*.ts" -l` and add
the four fields with values `null`, `"CommandOrControl+Alt+N"`,
`"CommandOrControl+Alt+Space"`, `true` to each fixture; `src/test/ipcMock.ts`
almost certainly owns the canonical one).

- [ ] **Step 6: Green** — `pnpm test --run && pnpm build` → PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/core/src/api.rs src/lib/ipc.ts src/test/ipcMock.ts
git add -u   # picks up any other test fixtures Step 5 had to extend
git commit -m "contract: settings carry the mic, hotkeys, and tray choice

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Settings rebuilt — six sections, a nav, prefilled values

**Files:**
- Rewrite: `src/components/Settings.tsx`
- Modify: `src/styles/panels.css` (settings layout additions)
- Modify: `src/App.tsx` (pass `theme` + `initialSection`)
- Modify: `src/lib/ipc.ts` (desktop-only block: `listInputDevices`)
- Create: `src/lib/desktop.ts`
- Test: `src/components/__tests__/settings.test.tsx` (nav cases added; existing label assertions must keep passing UNCHANGED)

**Interfaces:**
- Produces: `export type SettingsSection = "general" | "recording" | "hotkeys" | "ai" | "storage" | "updates"` (from `Settings.tsx`);
  `SettingsProps` becomes `{ onClose: () => void; theme: ReturnType<typeof useTheme>; initialSection?: SettingsSection }`;
  `src/lib/desktop.ts` exports `listInputDevices(): Promise<InputDevice[]>`
  with `export interface InputDevice { id: string; label: string; isDefault: boolean }`
  — returns `[]` when not on desktop or when the command fails (Task 8 lands
  the Rust side).
- Consumes: `Settings` fields from Task 3; `isDesktop()` from `transport.ts`;
  `Switch`, `Button`, `Notice` from `./ui`.
- Note: the **Hotkeys section renders in the nav in this task but its body
  is built in Task 5** — in this task its body is the two current accelerator
  values rendered read-only with `Kbd` (real data, no dead controls).

- [ ] **Step 1: Failing tests** — append to `settings.test.tsx`:

```tsx
describe("sectioned navigation", () => {
  it("shows six sections and opens General by default", async () => {
    const dialog = await openSettings(); // the file's existing helper that renders <Settings … /> and returns the dialog
    const nav = within(dialog).getByRole("navigation", { name: "Settings sections" });
    for (const label of ["General", "Recording", "Hotkeys", "Transcription & AI", "Storage", "Updates"]) {
      expect(within(nav).getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(within(dialog).getByRole("heading", { name: "General" })).toBeInTheDocument();
  });

  it("clicking a section shows that section's controls", async () => {
    const dialog = await openSettings();
    fireEvent.click(within(dialog).getByRole("button", { name: "Storage" }));
    expect(await within(dialog).findByLabelText("Where recordings are saved")).toBeInTheDocument();
  });

  it("initialSection opens the asked-for section", async () => {
    const dialog = await openSettings({ initialSection: "updates" });
    expect(within(dialog).getByRole("button", { name: "Check for updates" })).toBeInTheDocument();
  });

  it("close-to-tray switch reflects and writes the setting", async () => {
    const dialog = await openSettings();
    const sw = within(dialog).getByRole("switch", { name: "Close button hides to tray" });
    expect(sw).toHaveAttribute("data-state", "checked");
    fireEvent.click(sw);
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ closeToTray: false }),
    );
  });
});
```

Adapt `openSettings(...)`/`lastSetSettings()` to the file's existing helpers
(the file already renders Settings and inspects `set_settings` calls — reuse
its mechanism; if the helper takes no args today, extend it to spread extra
props). If NO such helper exists, define both in the test file exactly as:

```tsx
function SettingsHost(props: Partial<SettingsProps>) {
  const theme = useTheme();
  return <Settings onClose={vi.fn()} theme={theme} {...props} />;
}
async function openSettings(props: Partial<SettingsProps> = {}) {
  render(<SettingsHost {...props} />);
  return await screen.findByRole("dialog");
}
function lastSetSettings(): unknown {
  const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "set_settings");
  return (calls[calls.length - 1]?.[1] as { settings?: unknown })?.settings;
}
```

(`invokeMock` = however `src/test/ipcMock.ts` exposes the invoke spy — read
that file and use its exported handle.) Update the existing render sites for
the new `theme` prop the same way.

- [ ] **Step 2: Verify failure** — `pnpm test --run -- settings` → FAIL.

- [ ] **Step 3: Rebuild `Settings.tsx`.** Keep: every existing state hook,
handler (`updateSettings`, `commitStorage`, `commitBaseUrl`, `commitModel`,
`handlePull`, `handleCheckForUpdate`, `handleInstallUpdate`, `openLogFolder`,
`handleAutoRecordChange`, pull-progress polling, focus trap, `PullBar`,
`ollamaStatusLabel/Kind`, `KNOWN_APPS`, `POLICY_OPTIONS`) and every control
with its exact label. Change: the panel becomes a two-column layout — left
`<nav aria-label="Settings sections">` with six buttons, right a scrollable
pane rendering ONE active section. Shape:

```tsx
export type SettingsSection = "general" | "recording" | "hotkeys" | "ai" | "storage" | "updates";

const SECTIONS: Array<{ id: SettingsSection; label: string }> = [
  { id: "general", label: "General" },
  { id: "recording", label: "Recording" },
  { id: "hotkeys", label: "Hotkeys" },
  { id: "ai", label: "Transcription & AI" },
  { id: "storage", label: "Storage" },
  { id: "updates", label: "Updates" },
];

export interface SettingsProps {
  onClose: () => void;
  theme: ReturnType<typeof useTheme>;
  initialSection?: SettingsSection;
}
```

`const [section, setSection] = useState<SettingsSection>(initialSection ?? "general");`

Nav item classes (reuse the app's row pattern):
active `"bg-selected text-fg shadow-[inset_2px_0_0_var(--c-accent)]"`,
idle `"text-fg-muted hover:bg-hover hover:text-fg"` on
`"w-full rounded-[var(--radius-control)] px-3 py-1.5 text-left text-[13px] font-medium transition-colors"`.

Section contents (all controls verbatim from the current file, re-housed per
spec §5 table):

- **general**: heading "General"; theme preference select — label
  **"Theme"**, options `System` (`""`→`setPreference("system")`), `Light`,
  `Dark`, value `theme.preference`; close-to-tray `Switch` with label
  **"Close button hides to tray"** bound to `settings.closeToTray` via
  `updateSettings({ ...settings, closeToTray: v })`; the languages checkboxes
  block moved here unchanged (its labels are asserted: "Chinese (Mandarin)",
  "English", …).
- **recording**: heading "Recording"; microphone select — label
  **"Microphone"**, first option **"System default"** (value `""` →
  `inputDevice: null`), then one option per `listInputDevices()` result
  (loaded in an effect, desktop only), value `settings.inputDevice ?? ""`;
  the auto-record table unchanged (fieldsets per app, radio labels
  Ask/Always/Never, the Google Meet note); keep-WAV checkbox unchanged; the
  processing block (process-when-idle, minutes, require-AC) moved here with
  labels unchanged.
- **hotkeys**: heading "Hotkeys"; two read-only rows for now (Task 5 makes
  them recorders): row label **"Start / stop recording"** with hint
  **"Works anywhere, even with the window closed"**, and
  **"Show / hide Notetaker"** with hint **"Brings the window up from the
  tray"**; values rendered via `<Kbd>` from
  `formatAcceleratorParts(settings.hotkeyToggleRecord)` — for THIS task
  render the raw string split on `"+"` (Task 5 introduces the formatter and
  swaps it in).
- **ai**: heading "Transcription & AI"; tier select unchanged; speech engine
  select unchanged (label "Speech model"); model status: render
  `setupStatus()` results — when `missing.length > 0`, one row per missing
  model (`label` + `bytes` via the file-size formatting already used in
  FirstRun — copy that tiny helper in) with one **"Download"** `Button`
  calling `api.downloadModels()`; Ollama block + pull unchanged; base
  URL/model inputs under a `<details>` with `<summary>Advanced</summary>`,
  labels unchanged.
- **storage**: heading "Storage"; the storage-root input unchanged (label
  "Where recordings are saved"); the open-log-folder button unchanged.
- **updates**: heading "Updates"; the existing updates block unchanged.

Panel classes: replace the current `settings-panel` body wrapper with
`grid grid-cols-[168px_1fr]` inside the existing overlay/panel chrome; the
pane scrolls (`min-h-0 overflow-y-auto`). Add to `panels.css` under the
"sections and fields" comment:

```css
.settings-nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 12px 8px;
  border-right: 1px solid var(--c-border);
  background: color-mix(in srgb, var(--c-sunken) 60%, var(--c-app));
}
```

`src/lib/desktop.ts`:

```ts
/**
 * Desktop-shell-only commands. These are #[tauri::command]s on the app crate,
 * NOT part of runtime::COMMANDS — the LAN/web build must never call them,
 * which is why every function here checks isDesktop() itself.
 */
import { invoke } from "@tauri-apps/api/core";
import { isDesktop } from "./transport";

export interface InputDevice {
  id: string;
  label: string;
  isDefault: boolean;
}

export async function listInputDevices(): Promise<InputDevice[]> {
  if (!isDesktop()) return [];
  try {
    return await invoke<InputDevice[]>("list_input_devices");
  } catch {
    // The command lands with the native layer; an older shell answers with
    // an error. "System default" alone is the honest offer either way.
    return [];
  }
}
```

`App.tsx`: `{settingsOpen && <Settings onClose={…} theme={theme} initialSection={settingsSection} />}`
where `const [settingsSection, setSettingsSection] = useState<SettingsSection | undefined>(undefined)`
and the open-settings paths reset it to `undefined` (palette sets it in Task 6).

- [ ] **Step 4: Green** — `pnpm test --run -- settings` then full
`pnpm test --run && pnpm build`. Every pre-existing label assertion passes
unmodified; the new nav cases pass.

- [ ] **Step 5: Screenshot** — `bash scripts/shoot-ui.sh /tmp/overhaul-shots/task4`
then manually open Settings is not possible headless; instead assert via test
DOM only (screenshots of Settings come from the Windows pass, Task 12).

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings.tsx src/styles/panels.css src/App.tsx src/lib/desktop.ts src/lib/ipc.ts src/components/__tests__/settings.test.tsx
git commit -m "feat(ui): settings become a place — six sections, a nav, prefilled values

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Hotkey capture — the recorder rows

**Files:**
- Create: `src/lib/hotkeys.ts`
- Create: `src/lib/__tests__/hotkeys.test.ts`
- Create: `src/components/HotkeyField.tsx`
- Modify: `src/components/Settings.tsx` (hotkeys section uses HotkeyField)
- Test: `src/components/__tests__/settings.test.tsx` (recorder cases)

**Interfaces:**
- Produces: from `hotkeys.ts` —
  `formatAcceleratorParts(accel: string): string[]` (display keycaps, maps
  `CommandOrControl` → `⌘` on Mac / `Ctrl` otherwise via `modKey()`),
  `acceleratorFromEvent(e: { key: string; code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }): string | null`
  (null until a non-modifier key is down; `Ctrl/Cmd` → `CommandOrControl`).
  From `HotkeyField.tsx` — `<HotkeyField label hint value issue onChange />`
  with `issue: string | null` rendering the spec's verbatim conflict copy.
- Consumes: `Kbd`, `modKey` from `./ui`; Settings fields from Task 3.

- [ ] **Step 1: Failing tests** — `src/lib/__tests__/hotkeys.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { acceleratorFromEvent, formatAcceleratorParts } from "../hotkeys";

const ev = (over: Partial<Parameters<typeof acceleratorFromEvent>[0]>) => ({
  key: "n", code: "KeyN", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...over,
});

describe("acceleratorFromEvent", () => {
  it("builds CommandOrControl+Alt+N from ctrl+alt+n", () => {
    expect(acceleratorFromEvent(ev({ ctrlKey: true, altKey: true }))).toBe("CommandOrControl+Alt+N");
  });
  it("meta counts as CommandOrControl too", () => {
    expect(acceleratorFromEvent(ev({ metaKey: true, altKey: true }))).toBe("CommandOrControl+Alt+N");
  });
  it("returns null while only modifiers are down", () => {
    expect(acceleratorFromEvent(ev({ key: "Control", code: "ControlLeft", ctrlKey: true }))).toBeNull();
  });
  it("names Space and letters from code, not layout", () => {
    expect(acceleratorFromEvent(ev({ key: " ", code: "Space", ctrlKey: true, altKey: true }))).toBe("CommandOrControl+Alt+Space");
  });
  it("shift is carried", () => {
    expect(acceleratorFromEvent(ev({ ctrlKey: true, shiftKey: true }))).toBe("CommandOrControl+Shift+N");
  });
});

describe("formatAcceleratorParts", () => {
  it("splits and renames the modifier for display", () => {
    expect(formatAcceleratorParts("CommandOrControl+Alt+N")).toEqual(["Ctrl", "Alt", "N"]);
  });
});
```

(The Ctrl expectation is safe: jsdom's `navigator.platform` is not a Mac.)

- [ ] **Step 2: Verify failure** — `pnpm test --run -- hotkeys` → FAIL.

- [ ] **Step 3: Implement `src/lib/hotkeys.ts`**

```ts
/**
 * Tauri accelerator strings ("CommandOrControl+Alt+N") to and from the DOM.
 * `code` is the source of truth for the main key so a layout that moves
 * letters (AZERTY) still records the physical key the OS will match.
 */
import { modKey } from "../components/ui";

const MODIFIER_KEYS = new Set(["Control", "Meta", "Alt", "Shift", "AltGraph", "OS"]);

export function acceleratorFromEvent(e: {
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("CommandOrControl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  const main = mainKeyName(e.code, e.key);
  if (!main) return null;
  parts.push(main);
  // A bare letter with no modifier is not a global hotkey — it would swallow typing.
  if (parts.length === 1) return null;
  return parts.join("+");
}

function mainKeyName(code: string, key: string): string | null {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (code === "Space") return "Space";
  if (/^F\d{1,2}$/.test(code)) return code;
  const named: Record<string, string> = {
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Escape: "Escape", Enter: "Enter", Backspace: "Backspace", Delete: "Delete",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Tab: "Tab",
  };
  if (named[code]) return named[code];
  // Punctuation rows: fall back to the produced key when it is one printable char.
  if (key.length === 1 && key !== " ") return key.toUpperCase();
  return null;
}

export function formatAcceleratorParts(accel: string): string[] {
  return accel.split("+").map((p) => (p === "CommandOrControl" ? modKey() : p));
}
```

- [ ] **Step 4: `HotkeyField.tsx`**

```tsx
/**
 * One rebindable shortcut. Click → listening state → next chord is captured.
 * Escape cancels. The conflict message arrives from above (registration is
 * the native layer's job); this component only renders it, verbatim.
 */
import { useState } from "react";
import type { KeyboardEvent } from "react";
import { acceleratorFromEvent, formatAcceleratorParts } from "../lib/hotkeys";
import { Kbd } from "./ui";
import { cn } from "../lib/cn";

export function HotkeyField({
  label,
  hint,
  value,
  issue,
  onChange,
}: {
  label: string;
  hint: string;
  value: string;
  issue: string | null;
  onChange: (accelerator: string) => void;
}) {
  const [listening, setListening] = useState(false);

  function onKeyDown(e: KeyboardEvent<HTMLButtonElement>) {
    if (!listening) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setListening(false);
      return;
    }
    const accel = acceleratorFromEvent(e);
    if (accel) {
      onChange(accel);
      setListening(false);
    }
  }

  return (
    <div className="flex items-center justify-between gap-4 rounded-[var(--radius-control)] border border-border bg-raised px-3 py-2.5">
      <span className="min-w-0">
        <span className="block text-[13.5px] font-medium text-fg">{label}</span>
        <span className="block text-[12.5px] text-fg-muted">{hint}</span>
        {issue && (
          <span role="alert" className="block pt-1 text-[12.5px] text-error">
            {issue}
          </span>
        )}
      </span>
      <button
        type="button"
        aria-label={`Change shortcut: ${label}`}
        onClick={() => setListening(true)}
        onKeyDown={onKeyDown}
        onBlur={() => setListening(false)}
        className={cn(
          "flex shrink-0 items-center gap-1 rounded-[var(--radius-control)] px-1.5 py-1",
          listening && "outline outline-2 outline-accent shadow-[var(--glow-accent)]",
        )}
      >
        {listening ? (
          <span className="text-[12.5px] text-accent">Press the keys…</span>
        ) : (
          formatAcceleratorParts(value).map((part) => <Kbd key={part}>{part}</Kbd>)
        )}
      </button>
    </div>
  );
}
```

- [ ] **Step 5: Wire into Settings** — the hotkeys section body becomes:

```tsx
            <HotkeyField
              label="Start / stop recording"
              hint="Works anywhere, even with the window closed"
              value={settings.hotkeyToggleRecord}
              issue={hotkeyIssues?.toggleRecord ?? null}
              onChange={(a) => updateSettings({ ...settings, hotkeyToggleRecord: a })}
            />
            <HotkeyField
              label="Show / hide Notetaker"
              hint="Brings the window up from the tray"
              value={settings.hotkeyShowHide}
              issue={hotkeyIssues?.showHide ?? null}
              onChange={(a) => updateSettings({ ...settings, hotkeyShowHide: a })}
            />
```

Add to `SettingsProps`:
`hotkeyIssues?: { toggleRecord: string | null; showHide: string | null };`
(App passes it in Task 9; optional until then).

Settings test additions:

```tsx
  it("records a new start/stop hotkey from a chord", async () => {
    const dialog = await openSettings({ initialSection: "hotkeys" });
    const btn = within(dialog).getByRole("button", { name: "Change shortcut: Start / stop recording" });
    fireEvent.click(btn);
    fireEvent.keyDown(btn, { key: "r", code: "KeyR", ctrlKey: true, altKey: true });
    await waitFor(() =>
      expect(lastSetSettings()).toMatchObject({ hotkeyToggleRecord: "CommandOrControl+Alt+R" }),
    );
  });
```

- [ ] **Step 6: Green** — `pnpm test --run && pnpm build` → PASS.

- [ ] **Step 7: Commit**

```bash
git add src/lib/hotkeys.ts src/lib/__tests__/hotkeys.test.ts src/components/HotkeyField.tsx src/components/Settings.tsx src/components/__tests__/settings.test.tsx
git commit -m "feat(ui): rebindable hotkeys — press the keys, they are captured

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: The palette becomes find & jump

**Files:**
- Rewrite: `src/components/CommandPalette.tsx`
- Modify: `src/App.tsx` (palette props; settings deep link)
- Create: `src/components/__tests__/commandPalette.test.tsx`

**Interfaces:**
- Produces: `CommandPalette` props become
  `{ open; onOpenChange; recordings: RecordingRow[]; tasks: string[]; onSelectRecording(id); onSelectTask(name); onOpenSettings(section?: SettingsSection) }` —
  the `capture`, `actions`, `themeIsDark`, `canAsk` props are DELETED.
- Consumes: `SettingsSection`, `SECTIONS` labels from Task 4 (import the type;
  re-declare the label list locally as
  `[{ id, label }]` matching Task 4's `SECTIONS` exactly).
- App: `onSelectTask` = `lib.setView({ kind: "task", name })`;
  `onOpenSettings(section)` = `setSettingsSection(section); setSettingsOpen(true)`.

- [ ] **Step 1: Failing test** — `src/components/__tests__/commandPalette.test.tsx`:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CommandPalette } from "../CommandPalette";
import type { RecordingRow } from "../../lib/ipc";

const row: RecordingRow = {
  id: "r1", title: "Accounting sync", task: null, created: "2026-08-04T10:00:00Z",
  durationS: 60, mode: "meeting", status: "ready", suggestedTask: null,
  suggestedTitle: null, hasNotes: false, error: null, captureNote: null,
};

function renderPalette(over: Partial<Parameters<typeof CommandPalette>[0]> = {}) {
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    recordings: [row],
    tasks: ["Entrepreneurship"],
    onSelectRecording: vi.fn(),
    onSelectTask: vi.fn(),
    onOpenSettings: vi.fn(),
    ...over,
  };
  render(<CommandPalette {...props} />);
  return props;
}

describe("CommandPalette", () => {
  it("never offers recording controls", () => {
    renderPalette();
    expect(screen.queryByText(/Record a meeting/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Record in person/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Stop recording/)).not.toBeInTheDocument();
    expect(screen.queryByText(/mode/i)).not.toBeInTheDocument();
  });

  it("jumps to a recording", () => {
    const p = renderPalette();
    fireEvent.click(screen.getByText("Accounting sync"));
    expect(p.onSelectRecording).toHaveBeenCalledWith("r1");
    expect(p.onOpenChange).toHaveBeenCalledWith(false);
  });

  it("jumps to a task", () => {
    const p = renderPalette();
    fireEvent.click(screen.getByText("Entrepreneurship"));
    expect(p.onSelectTask).toHaveBeenCalledWith("Entrepreneurship");
  });

  it("deep-links into a settings section", () => {
    const p = renderPalette();
    fireEvent.click(screen.getByText("Hotkeys"));
    expect(p.onOpenSettings).toHaveBeenCalledWith("hotkeys");
  });
});
```

- [ ] **Step 2: Verify failure** — `pnpm test --run -- commandPalette` → FAIL.

- [ ] **Step 3: Rewrite `CommandPalette.tsx`** — keep the file's dialog
shell, Ctrl/Cmd+K effect, `Row` subcomponent, and cmdk usage exactly; replace
the header comment's claims about "things to do" with find & jump; change the
input placeholder to `"Jump to…"`; delete the `PaletteActions` interface and
the "Do" group; render three groups (each with the existing group-heading
classes):

```tsx
              <Command.Group heading="Recordings" …>   {/* as today, unchanged rows */}
              <Command.Group heading="Tasks" …>
                {tasks.map((task) => (
                  <Row key={task} icon={<FolderOpen size={14} />} onSelect={() => run(() => onSelectTask(task))}>
                    {task}
                  </Row>
                ))}
              </Command.Group>
              <Command.Group heading="Settings" …>
                {PALETTE_SECTIONS.map((s) => (
                  <Row key={s.id} icon={<SettingsIcon size={14} />} value={`settings ${s.label}`}
                       onSelect={() => run(() => onOpenSettings(s.id))} hint="settings ›">
                    {s.label}
                  </Row>
                ))}
              </Command.Group>
```

with (import: `import type { SettingsSection } from "./Settings";`)

```tsx
const PALETTE_SECTIONS: Array<{ id: SettingsSection; label: string }> = [
  { id: "general", label: "General" },
  { id: "recording", label: "Recording" },
  { id: "hotkeys", label: "Hotkeys" },
  { id: "ai", label: "Transcription & AI" },
  { id: "storage", label: "Storage" },
  { id: "updates", label: "Updates" },
];
```

Imports: `FolderOpen, Settings as SettingsIcon, FileText` from lucide; drop
`Circle, Mic, Moon, Square, Sun, Sparkles`. The empty-state line and the
Recordings group stay as they are today.

- [ ] **Step 4: Rewire `App.tsx`** — the `<CommandPalette …>` call site loses
`capture`, `actions`, `themeIsDark`, `canAsk` and gains
`tasks={lib.tasks}`, `onSelectTask={(name) => lib.setView({ kind: "task", name })}`,
`onOpenSettings={(section) => { setSettingsSection(section); setSettingsOpen(true); }}`.

- [ ] **Step 5: Green** — `pnpm test --run && pnpm build` → PASS (no existing
test asserted the removed rows — verified by grep during planning).

- [ ] **Step 6: Commit**

```bash
git add src/components/CommandPalette.tsx src/components/__tests__/commandPalette.test.tsx src/App.tsx
git commit -m "feat(ui): the palette finds and jumps; recording keeps its two homes

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: Echo — the mark, the icons, the empty state

**Files:**
- Create: `src-tauri/icons/source/echo.svg`, `echo-tray-idle.svg`,
  `echo-tray-recording.svg`, `echo-tray-paused.svg`
- Create: `scripts/render-icons.sh`
- Regenerate: `src-tauri/icons/*` (via `pnpm tauri icon`)
- Create: `src-tauri/icons/tray/idle.png`, `recording.png`, `paused.png`
- Create: `src/components/EchoMark.tsx`
- Replace: `public/notetaker.svg` (web favicon becomes Echo)
- Modify: `src/components/Sidebar.tsx` (empty state), `src/App.tsx` (pass hotkey label)
- Test: `src/components/__tests__/sidebar.test.tsx` (empty-state case)

**Interfaces:**
- Produces: `<EchoMark size={number} dim?: boolean />` (inline SVG React
  component); Sidebar prop `recordHotkeyLabel: string | null`; tray PNG paths
  consumed by Task 8 exactly as listed above.

- [ ] **Step 1: `src-tauri/icons/source/echo.svg`** — exact content:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 120 120">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#8b72ff"/><stop offset="1" stop-color="#4dd6ff"/>
    </linearGradient>
    <filter id="glow" x="-40%" y="-40%" width="180%" height="180%">
      <feGaussianBlur stdDeviation="4" result="b"/>
      <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  </defs>
  <rect x="0" y="0" width="120" height="120" rx="27" fill="#0b0b12"/>
  <rect x="0" y="0" width="120" height="120" rx="27" fill="url(#bg)" opacity=".16"/>
  <g filter="url(#glow)">
    <path d="M34 78 V54 a26 26 0 0 1 52 0 V78 a6 6 0 0 1 -9 5 l-4 -2.5 a6 6 0 0 0 -6.5 0 l-3 2 a6 6 0 0 1 -7 0 l-3 -2 a6 6 0 0 0 -6.5 0 l-4 2.5 a6 6 0 0 1 -9 -5 Z" fill="url(#bg)"/>
  </g>
  <ellipse cx="51" cy="50" rx="3.2" ry="4.8" fill="#0b0b12"/>
  <ellipse cx="69" cy="50" rx="3.2" ry="4.8" fill="#0b0b12"/>
  <circle cx="52" cy="48.4" r="1" fill="#efeffa" opacity=".9"/>
  <circle cx="70" cy="48.4" r="1" fill="#efeffa" opacity=".9"/>
  <g stroke="#0b0b12" stroke-width="3.6" stroke-linecap="round">
    <line x1="51" y1="66" x2="51" y2="70"/><line x1="58" y1="63" x2="58" y2="73"/><line x1="65" y1="65" x2="65" y2="71"/>
  </g>
</svg>
```

- [ ] **Step 2: Tray silhouettes** — `echo-tray-idle.svg` exact content
(32×32 canvas, silhouette only, no background):

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 120 120">
  <path d="M34 78 V54 a26 26 0 0 1 52 0 V78 a6 6 0 0 1 -9 5 l-4 -2.5 a6 6 0 0 0 -6.5 0 l-3 2 a6 6 0 0 1 -7 0 l-3 -2 a6 6 0 0 0 -6.5 0 l-4 2.5 a6 6 0 0 1 -9 -5 Z" fill="#b9b7d0"/>
  <ellipse cx="51" cy="50" rx="3.2" ry="4.8" fill="#0b0b12"/>
  <ellipse cx="69" cy="50" rx="3.2" ry="4.8" fill="#0b0b12"/>
  <g stroke="#0b0b12" stroke-width="3.6" stroke-linecap="round">
    <line x1="51" y1="66" x2="51" y2="70"/><line x1="58" y1="63" x2="58" y2="73"/><line x1="65" y1="65" x2="65" y2="71"/>
  </g>
</svg>
```

`echo-tray-recording.svg`: same as idle but body `fill="#efeffa"` and, before
`</svg>`, add
`<circle cx="92" cy="92" r="18" fill="#ff5c51" stroke="#0b0b12" stroke-width="6"/>`.
`echo-tray-paused.svg`: same as recording but the circle `fill="#f5b84d"`.

- [ ] **Step 3: `scripts/render-icons.sh` (0755)**

```bash
#!/usr/bin/env bash
# Renders Echo SVGs to PNGs and regenerates the full Tauri icon set.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src-tauri/icons/source"
CHROME=$(ls -d ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome | head -1)

shot() { # $1 svg  $2 out.png  $3 size
  "$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
    --default-background-color=00000000 --window-size="$3,$3" \
    --screenshot="$2" "file://$1"
}

shot "$SRC/echo.svg" "$SRC/echo-1024.png" 1024
mkdir -p "$ROOT/src-tauri/icons/tray"
shot "$SRC/echo-tray-idle.svg"      "$ROOT/src-tauri/icons/tray/idle.png"      32
shot "$SRC/echo-tray-recording.svg" "$ROOT/src-tauri/icons/tray/recording.png" 32
shot "$SRC/echo-tray-paused.svg"    "$ROOT/src-tauri/icons/tray/paused.png"    32
cd "$ROOT" && pnpm tauri icon "$SRC/echo-1024.png"
echo "icons regenerated"
```

- [ ] **Step 4: Run it** — `bash scripts/render-icons.sh`
Expected: `src-tauri/icons/` regenerated (32x32.png, 128x128.png, icon.ico,
icon.icns, Square*.png, StoreLogo.png all newer than the run start), three
tray PNGs exist. Check transparency:
`python3 -c "print(open('src-tauri/icons/tray/idle.png','rb').read(26)[24:26].hex())"` →
`0806` (8-bit RGBA). Open `src-tauri/icons/128x128.png` and confirm: dark
squircle, aurora ghost, waveform mouth (compare to Plate 01 pick A).

- [ ] **Step 5: `EchoMark.tsx`** — inline component reusing the same paths:

```tsx
/** Echo, the listener — inline for empty states. Decorative only. */
export function EchoMark({ size = 96, dim = false }: { size?: number; dim?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 120 120" aria-hidden="true"
         style={dim ? { opacity: 0.45 } : undefined}>
      <defs>
        <linearGradient id="echo-bg" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="var(--c-accent)" />
          <stop offset="1" stopColor="var(--c-accent-2)" />
        </linearGradient>
      </defs>
      <path d="M34 78 V54 a26 26 0 0 1 52 0 V78 a6 6 0 0 1 -9 5 l-4 -2.5 a6 6 0 0 0 -6.5 0 l-3 2 a6 6 0 0 1 -7 0 l-3 -2 a6 6 0 0 0 -6.5 0 l-4 2.5 a6 6 0 0 1 -9 -5 Z" fill="url(#echo-bg)"/>
      <ellipse cx="51" cy="50" rx="3.2" ry="4.8" fill="var(--c-app)"/>
      <ellipse cx="69" cy="50" rx="3.2" ry="4.8" fill="var(--c-app)"/>
      <g stroke="var(--c-app)" strokeWidth="3.6" strokeLinecap="round">
        <line x1="51" y1="66" x2="51" y2="70"/><line x1="58" y1="63" x2="58" y2="73"/><line x1="65" y1="65" x2="65" y2="71"/>
      </g>
    </svg>
  );
}
```

- [ ] **Step 6: Sidebar empty state + favicon** — Sidebar gains prop
`recordHotkeyLabel: string | null`; the empty paragraph (currently "Nothing
here yet. Hit record and start typing…") becomes:

```tsx
                <div className="flex flex-col items-center gap-2 px-2 py-6 text-center">
                  <EchoMark size={72} dim />
                  <p className="text-[12px] leading-relaxed text-fg-faint">
                    {recordHotkeyLabel
                      ? `Nothing here yet — hit record, or press ${recordHotkeyLabel}.`
                      : "Nothing here yet — hit record."}
                  </p>
                </div>
```

App passes
`recordHotkeyLabel={isDesktop() && appSettings ? formatAcceleratorParts(appSettings.hotkeyToggleRecord).join("+") : null}`
— App loads settings once for this + later native needs (extend the existing
ipc import to `import { api, type CaptureStatus, type Settings, type SetupStatus } from "./lib/ipc";`):
`const [appSettings, setAppSettings] = useState<Settings | null>(null);`
with `useEffect(() => { api.getSettings().then(setAppSettings).catch(() => setAppSettings(null)); }, [settingsOpen]);`
(refetch when Settings closes so a rebind shows up). Replace
`public/notetaker.svg` content with the `echo.svg` content from Step 1
(unchanged — browsers scale it). Update the sidebar empty-state test to expect
the new copy (`/Nothing here yet — hit record/`).

- [ ] **Step 7: Green + commit**

```bash
pnpm test --run && pnpm build
git add src-tauri/icons scripts/render-icons.sh src/components/EchoMark.tsx src/components/Sidebar.tsx src/App.tsx public/notetaker.svg src/components/__tests__/sidebar.test.tsx
git commit -m "feat: Echo — the app icon, tray marks, favicon, and empty state

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: Native layer A — plugins, tray, close-to-tray, device list

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`,
  `src-tauri/capabilities/default.json`, `package.json`
- Create: `src-tauri/src/tray.rs`
- Modify: `src/App.tsx`, `src/hooks/useCapture.ts` (tray status effect),
  `src/lib/desktop.ts` (`setTrayStatus`)
- Test: frontend parts in `src/components/__tests__/capture.test.tsx`
  (tray-status mapping unit test in `src/lib/__tests__/desktop.test.ts`)

**Interfaces:**
- Produces (Rust commands, app crate only): `list_input_devices` returning
  `Vec<InputDevice { id: String, label: String, is_default: bool }>`
  (serde camelCase → `isDefault`); `set_tray_status(state: String)` accepting
  exactly `"idle" | "recording" | "paused"`.
- Produces (TS): `desktop.ts` adds
  `trayStateFor(state: CaptureState): "idle" | "recording" | "paused"`
  (`finishing` → `"idle"`) and
  `setTrayStatus(state: CaptureState): Promise<void>` (isDesktop-guarded,
  errors swallowed).
- Produces (events, Rust→JS): `"tray-toggle-recording"`,
  `"tray-open-settings"`, `"close-requested"` — the webview maps toggle onto
  start vs stop from its own capture state.
- Tray menu labels, exact: `"Start recording"` / `"Stop recording"` (by
  state), `"Open Notetaker"`, `"Settings"`, `"Quit Notetaker"`.

- [ ] **Step 1: Frontend unit test first** — `src/lib/__tests__/desktop.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { trayStateFor } from "../desktop";

describe("trayStateFor", () => {
  it("maps capture states onto the three tray icons", () => {
    expect(trayStateFor("idle")).toBe("idle");
    expect(trayStateFor("recording")).toBe("recording");
    expect(trayStateFor("paused")).toBe("paused");
    // Finishing is not capturing: the red dot must not linger after stop.
    expect(trayStateFor("finishing")).toBe("idle");
  });
});
```

Run `pnpm test --run -- desktop` → FAIL. Implement in `desktop.ts`:

```ts
import type { CaptureState } from "./ipc";

export type TrayState = "idle" | "recording" | "paused";

export function trayStateFor(state: CaptureState): TrayState {
  if (state === "recording") return "recording";
  if (state === "paused") return "paused";
  return "idle";
}

let lastTray: TrayState | null = null;

export async function setTrayStatus(state: CaptureState): Promise<void> {
  if (!isDesktop()) return;
  const next = trayStateFor(state);
  if (next === lastTray) return;
  lastTray = next;
  try {
    await invoke("set_tray_status", { state: next });
  } catch {
    // An older shell has no tray; the app must not care.
  }
}
```

→ PASS. In `App.tsx` add
`useEffect(() => { void setTrayStatus(capture.status.state); }, [capture.status.state]);`

- [ ] **Step 2: JS plugin deps**

```bash
pnpm add @tauri-apps/plugin-global-shortcut @tauri-apps/plugin-autostart @tauri-apps/plugin-dialog
```

(global-shortcut/autostart are consumed in Task 9; installing here keeps one
lockfile change.) `pnpm build` → clean.

- [ ] **Step 3: Rust deps** — in `src-tauri/Cargo.toml` `[dependencies]` add:

```toml
tauri-plugin-global-shortcut = "2"
tauri-plugin-autostart = "2"
tauri-plugin-single-instance = "2"
tauri-plugin-window-state = "2"
tauri-plugin-dialog = "2"
```

And mirror the workspace's cpal line
(`grep -n "^cpal" src-tauri/platform/Cargo.toml` → copy that exact
version/features into the app crate's `[dependencies]`).

- [ ] **Step 4: `src-tauri/src/tray.rs`** — complete file, exactly this:

```rust
//! The tray: Echo in the corner, state at a glance, essentials on right-click.
//!
//! The frontend drives state via `set_tray_status` (it already polls capture
//! status for the record bar). Open/Quit act natively; Start/Stop/Settings are
//! forwarded to the webview, which owns the capture flow.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};

pub const TRAY_ID: &str = "main-tray";

/// The one menu item whose label changes with capture state.
pub struct TrayHandles<R: Runtime> {
    toggle: MenuItem<R>,
}

fn icon_bytes(state: &str) -> &'static [u8] {
    match state {
        "recording" => include_bytes!("../icons/tray/recording.png"),
        "paused" => include_bytes!("../icons/tray/paused.png"),
        _ => include_bytes!("../icons/tray/idle.png"),
    }
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<TrayIcon<R>> {
    let toggle = MenuItem::with_id(app, "tray-toggle", "Start recording", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "tray-open", "Open Notetaker", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "tray-settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Notetaker", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &open, &sep, &settings, &quit])?;
    app.manage(TrayHandles { toggle });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(icon_bytes("idle"))?)
        .tooltip("Notetaker")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // Left-click only: right-click opens the menu and must not also
            // pop the window.
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-toggle" => {
                // The webview decides start vs stop from its own state.
                show_main(app);
                let _ = app.emit("tray-toggle-recording", ());
            }
            "tray-open" => show_main(app),
            "tray-settings" => {
                show_main(app);
                let _ = app.emit("tray-open-settings", ());
            }
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .build(app)
}

/// Called by the frontend whenever capture state changes.
pub fn set_state<R: Runtime>(app: &AppHandle<R>, state: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(icon) = Image::from_bytes(icon_bytes(state)) {
            let _ = tray.set_icon(Some(icon));
        }
        let _ = tray.set_tooltip(Some(match state {
            "recording" => "Notetaker — recording",
            "paused" => "Notetaker — paused",
            _ => "Notetaker",
        }));
    }
    if let Some(handles) = app.try_state::<TrayHandles<R>>() {
        let label = if state == "recording" || state == "paused" {
            "Stop recording"
        } else {
            "Start recording"
        };
        let _ = handles.toggle.set_text(label);
    }
}
```

If CI's compiler rejects a tray/menu API name (field, method, or import path —
these shifted across Tauri 2 minors), the compiler's suggestion is
authoritative: adjust to it, keep the behavior identical, and say what moved
in the commit body.

- [ ] **Step 5: Wire `lib.rs`** — in `run()`:

after the existing `.plugin(tauri_plugin_updater…)` line add:

```rust
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
```

in `.setup(…)` after `app.manage(runtime);` add:

```rust
            tray::build(&app.handle().clone())?;
```

add module + close interception (top of file: `mod tray;`; in the builder,
before `.run(…)`):

```rust
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // The frontend owns the decision (setting + first-time note).
                api.prevent_close();
                let _ = window.emit("close-requested", ());
            }
        })
```

two new commands (place near the other `#[tauri::command]` fns; follow the
file's existing command style):

```rust
#[tauri::command]
fn set_tray_status(app: tauri::AppHandle, state: String) {
    tray::set_state(&app, &state);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InputDevice {
    id: String,
    label: String,
    is_default: bool,
}

#[tauri::command]
fn list_input_devices() -> Vec<InputDevice> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();
    host.input_devices()
        .map(|devices| {
            devices
                .filter_map(|d| d.name().ok())
                .map(|name| InputDevice {
                    id: name.clone(),
                    label: name.clone(),
                    is_default: name == default_name,
                })
                .collect()
        })
        .unwrap_or_default()
}
```

and register both at the END of `generate_handler![ …, set_tray_status, list_input_devices ]`.

- [ ] **Step 6: Capabilities** — `src-tauri/capabilities/default.json`
permissions array becomes exactly:

```json
  "permissions": [
    "core:default",
    "opener:default",
    "process:default",
    "updater:default",
    "dialog:allow-open",
    "autostart:allow-enable",
    "autostart:allow-disable",
    "autostart:allow-is-enabled",
    "global-shortcut:allow-register",
    "global-shortcut:allow-unregister",
    "global-shortcut:allow-unregister-all",
    "global-shortcut:allow-is-registered",
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-is-maximized",
    "core:window:allow-close",
    "core:window:allow-hide",
    "core:window:allow-show",
    "core:window:allow-set-focus",
    "core:window:allow-start-dragging",
    "core:event:allow-listen",
    "core:event:allow-emit"
  ]
```

(If `pnpm build`/CI reports an unknown permission string, fix the string to
the name the error suggests — permission names are machine-checked, so the
error is authoritative. Do not delete entries to make errors go away.)

- [ ] **Step 7: Frontend close/tray handling** — in `App.tsx`. Imports:
`listen` from `@tauri-apps/api/event`, `getCurrentWindow` from
`@tauri-apps/api/window`, `exit` from `@tauri-apps/plugin-process`; extend
the `./components/ui` import with `Button, Dialog`.

The listeners mount ONCE and read live values through refs — depending on
`capture` directly would tear the listeners down on every status poll tick:

```tsx
  const [showTrayNote, setShowTrayNote] = useState(false);
  const [showQuitGuard, setShowQuitGuard] = useState(false);

  // Live values for mount-once listeners and OS hotkeys. Without these,
  // effect deps on `capture` would re-run every poll tick.
  const captureRef = useRef(capture);
  captureRef.current = capture;
  const stopAndOpenRef = useRef(stopAndOpen);
  stopAndOpenRef.current = stopAndOpen;
  const closeToTrayRef = useRef(true);
  closeToTrayRef.current = appSettings?.closeToTray ?? true;

  useEffect(() => {
    if (!isDesktop()) return;
    const unlistens = [
      listen("close-requested", async () => {
        const live =
          captureRef.current.status.state === "recording" ||
          captureRef.current.status.state === "paused";
        if (!closeToTrayRef.current) {
          if (live) {
            // Never let quit eat a take: stop-and-save is offered first.
            setShowQuitGuard(true);
            return;
          }
          await exit(0);
          return;
        }
        let explained = false;
        try { explained = localStorage.getItem("notetaker.trayExplained") === "1"; } catch { /* ignore */ }
        if (!explained) {
          setShowTrayNote(true);
          return;
        }
        await getCurrentWindow().hide();
      }),
      listen("tray-toggle-recording", () => {
        const c = captureRef.current;
        if (c.status.state === "recording" || c.status.state === "paused") {
          void stopAndOpenRef.current();
        } else if (c.status.state === "idle") {
          c.start("meeting", "");
        } // finishing: ignore — the recording is still landing.
      }),
      listen("tray-open-settings", () => setSettingsOpen(true)),
    ];
    return () => {
      unlistens.forEach((p) => p.then((u) => u()));
    };
  }, []);
```

and the one-time dialog (uses `Dialog` from `./components/ui`), rendered next
to the other overlays:

```tsx
      <Dialog
        open={showTrayNote}
        onOpenChange={(o) => setShowTrayNote(o)}
        title="Still running"
        description="Notetaker keeps running here in the tray so meeting detection and your recording hotkey still work. Quit completely from the tray icon."
      >
        <div className="flex justify-end gap-2">
          <Button
            variant="secondary"
            onClick={async () => { await exit(0); }}
          >
            Quit instead
          </Button>
          <Button
            variant="primary"
            onClick={async () => {
              try { localStorage.setItem("notetaker.trayExplained", "1"); } catch { /* ignore */ }
              setShowTrayNote(false);
              await getCurrentWindow().hide();
            }}
          >
            Got it
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={showQuitGuard}
        onOpenChange={setShowQuitGuard}
        title="Recording in progress"
        description="Quitting now would end the recording. It will be stopped and saved first."
      >
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setShowQuitGuard(false)}>
            Keep recording
          </Button>
          <Button
            variant="primary"
            onClick={async () => {
              await stopAndOpenRef.current();
              await exit(0);
            }}
          >
            Stop and save, then quit
          </Button>
        </div>
      </Dialog>
```

Guard EVERY `@tauri-apps/*` import usage behind `isDesktop()` at the call
site (the imports themselves are safe — the modules are plain JS until
invoked). `pnpm test --run` must stay green: jsdom runs the web path where
the effect returns immediately.

- [ ] **Step 8: Local checks + CI**

```bash
pnpm test --run && pnpm build
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core
git add -A && git commit -m "feat(native): the tray — Echo in the corner, close hides, state at a glance

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push -u origin claude/app-ui-ux-overhaul-96e4c6   # if the guard blocks: STOP, surface it
gh run list --branch claude/app-ui-ux-overhaul-96e4c6 --limit 3
gh run watch <newest-run-id> --exit-status
```
Expected: CI green on all three OS jobs (the Windows job proves the app crate
+ tray compile). CI red → read the log, fix, push again; two failed fix
attempts → stop and report.

---

### Task 9: Native layer B — global hotkeys, autostart, folder picker

**Files:**
- Create: `src/hooks/useGlobalHotkeys.ts`
- Create: `src/hooks/__tests__/useGlobalHotkeys.test.tsx`
- Modify: `src/App.tsx` (hook + hotkeyIssues plumb), `src/components/Settings.tsx`
  (autostart switch + Choose folder), `src/lib/desktop.ts` (pickFolder, autostart wrappers)

**Interfaces:**
- Produces: `useGlobalHotkeys(opts: { enabled: boolean; toggleRecord: string; showHide: string; onToggleRecord(): void }): { issues: { toggleRecord: string | null; showHide: string | null } }`;
  `desktop.ts` adds `pickFolder(): Promise<string | null>`,
  `getAutostart(): Promise<boolean | null>` (null = unavailable),
  `setAutostart(on: boolean): Promise<void>`.
- Conflict copy, verbatim (spec §5): `"That combination is taken by another app — pick a different one."`
- Settings additions: General gains Switch labeled
  **"Start Notetaker with Windows"**; Storage gains Button **"Choose folder…"**.

- [ ] **Step 1: Failing hook test** — `src/hooks/__tests__/useGlobalHotkeys.test.tsx`:

```tsx
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const register = vi.fn();
const unregisterAll = vi.fn();
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: (...a: unknown[]) => register(...a),
  unregisterAll: (...a: unknown[]) => unregisterAll(...a),
}));
vi.mock("../../lib/transport", async (orig) => ({
  ...(await orig()),
  isDesktop: () => true,
}));

import { useGlobalHotkeys } from "../useGlobalHotkeys";

describe("useGlobalHotkeys", () => {
  beforeEach(() => {
    register.mockReset().mockResolvedValue(undefined);
    unregisterAll.mockReset().mockResolvedValue(undefined);
  });

  it("registers both accelerators", async () => {
    renderHook(() =>
      useGlobalHotkeys({
        enabled: true,
        toggleRecord: "CommandOrControl+Alt+N",
        showHide: "CommandOrControl+Alt+Space",
        onToggleRecord: vi.fn(),
      }),
    );
    await waitFor(() => expect(register).toHaveBeenCalledTimes(2));
    expect(register.mock.calls.map((c) => c[0])).toEqual([
      "CommandOrControl+Alt+N",
      "CommandOrControl+Alt+Space",
    ]);
  });

  it("surfaces a registration failure as the spec's copy", async () => {
    register.mockRejectedValueOnce(new Error("already registered"));
    const { result } = renderHook(() =>
      useGlobalHotkeys({
        enabled: true,
        toggleRecord: "CommandOrControl+Alt+N",
        showHide: "CommandOrControl+Alt+Space",
        onToggleRecord: vi.fn(),
      }),
    );
    await waitFor(() =>
      expect(result.current.issues.toggleRecord).toBe(
        "That combination is taken by another app — pick a different one.",
      ),
    );
    expect(result.current.issues.showHide).toBeNull();
  });
});
```

Run → FAIL (module missing).

- [ ] **Step 2: Implement `useGlobalHotkeys.ts`**

```ts
/**
 * OS-wide shortcuts. Registered from the webview because the webview owns the
 * capture flow and the settings; the window being hidden does not stop its JS.
 * Failures are surfaced, never silent — a hotkey that quietly does nothing is
 * indistinguishable from a broken app.
 */
import { useEffect, useState } from "react";
import { isDesktop } from "../lib/transport";

const CONFLICT_COPY = "That combination is taken by another app — pick a different one.";

export interface HotkeyIssues {
  toggleRecord: string | null;
  showHide: string | null;
}

export function useGlobalHotkeys({
  enabled,
  toggleRecord,
  showHide,
  onToggleRecord,
}: {
  enabled: boolean;
  toggleRecord: string;
  showHide: string;
  onToggleRecord: () => void;
}): { issues: HotkeyIssues } {
  const [issues, setIssues] = useState<HotkeyIssues>({ toggleRecord: null, showHide: null });

  useEffect(() => {
    if (!enabled || !isDesktop()) return;
    let cancelled = false;

    (async () => {
      const { register, unregisterAll } = await import("@tauri-apps/plugin-global-shortcut");
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await unregisterAll().catch(() => undefined);
      if (cancelled) return;

      const next: HotkeyIssues = { toggleRecord: null, showHide: null };
      try {
        await register(toggleRecord, (e) => {
          if (e.state === "Pressed") onToggleRecord();
        });
      } catch {
        next.toggleRecord = CONFLICT_COPY;
      }
      try {
        await register(showHide, async (e) => {
          if (e.state !== "Pressed") return;
          const win = getCurrentWindow();
          if (await win.isVisible()) {
            await win.hide();
          } else {
            await win.show();
            await win.unminimize();
            await win.setFocus();
          }
        });
      } catch {
        next.showHide = CONFLICT_COPY;
      }
      if (!cancelled) setIssues(next);
    })();

    return () => {
      cancelled = true;
      void import("@tauri-apps/plugin-global-shortcut")
        .then((m) => m.unregisterAll())
        .catch(() => undefined);
    };
  }, [enabled, toggleRecord, showHide, onToggleRecord]);

  return { issues };
}
```

Tests → PASS (`pnpm test --run -- useGlobalHotkeys`).

- [ ] **Step 3: Wire App** — uses the `captureRef`/`stopAndOpenRef` refs from
Task 8 Step 7 so the callback stays identity-stable (deps on `capture` would
unregister and re-register the OS hotkeys on every status poll tick):

```tsx
  const onToggleRecordHotkey = useCallback(() => {
    const c = captureRef.current;
    if (c.status.state === "recording" || c.status.state === "paused") {
      void stopAndOpenRef.current();
    } else if (c.status.state === "idle") {
      c.start("meeting", "");
    } // finishing: ignore — the recording is still landing.
  }, []);

  const hotkeys = useGlobalHotkeys({
    enabled: appSettings !== null,
    toggleRecord: appSettings?.hotkeyToggleRecord ?? "CommandOrControl+Alt+N",
    showHide: appSettings?.hotkeyShowHide ?? "CommandOrControl+Alt+Space",
    onToggleRecord: onToggleRecordHotkey,
  });
```

Pass `hotkeyIssues={hotkeys.issues}` into `<Settings …>`.
(The `settingsOpen`-keyed refetch of `appSettings` from Task 7 makes a rebind
re-register on Settings close — note this dependency works because
`appSettings` object identity changes.)

- [ ] **Step 4: Autostart + folder picker in `desktop.ts`**

```ts
export async function pickFolder(): Promise<string | null> {
  if (!isDesktop()) return null;
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  } catch {
    return null;
  }
}

export async function getAutostart(): Promise<boolean | null> {
  if (!isDesktop()) return null;
  try {
    const { isEnabled } = await import("@tauri-apps/plugin-autostart");
    return await isEnabled();
  } catch {
    return null;
  }
}

export async function setAutostart(on: boolean): Promise<void> {
  if (!isDesktop()) return;
  try {
    const { enable, disable } = await import("@tauri-apps/plugin-autostart");
    if (on) await enable();
    else await disable();
  } catch {
    // Shown state re-reads on next open; a failed write is visible then.
  }
}
```

Settings General section adds (desktop only, after the close-to-tray switch):

```tsx
              {isDesktop() && autostart !== null && (
                <div className="flex items-center justify-between gap-4">
                  <span className="text-[13.5px] text-fg">Start Notetaker with Windows</span>
                  <Switch
                    label="Start Notetaker with Windows"
                    checked={autostart}
                    onCheckedChange={(v) => {
                      setAutostartState(v);
                      void setAutostart(v);
                    }}
                  />
                </div>
              )}
```

with `const [autostart, setAutostartState] = useState<boolean | null>(null);`
and an effect `useEffect(() => { getAutostart().then(setAutostartState); }, []);`.
First-run default-on: in `App.tsx`, one effect —

```tsx
  useEffect(() => {
    if (!isDesktop()) return;
    try {
      if (localStorage.getItem("notetaker.autostartInit") === "1") return;
      localStorage.setItem("notetaker.autostartInit", "1");
    } catch { return; }
    void setAutostart(true);
  }, []);
```

Storage section adds next to the input (desktop only):

```tsx
                  {isDesktop() && (
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={async () => {
                        const dir = await pickFolder();
                        if (dir && settings) updateSettings({ ...settings, storageRoot: dir });
                      }}
                    >
                      Choose folder…
                    </Button>
                  )}
```

- [ ] **Step 5: Green + CI**

```bash
pnpm test --run && pnpm build
git add -A && git commit -m "feat(native): global hotkeys, start-with-windows, and a real folder picker

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push && gh run watch $(gh run list --branch claude/app-ui-ux-overhaul-96e4c6 --limit 1 --json databaseId -q '.[0].databaseId') --exit-status
```

---

### Task 10: The custom titlebar

**Files:**
- Modify: `src-tauri/tauri.conf.json` (window entry), `src/App.tsx` (header)
- Create: `src/components/WindowControls.tsx`
- Test: `src/components/__tests__/windowControls.test.tsx`

**Interfaces:**
- Produces: `<WindowControls />` (desktop-only, self-guarding).
- tauri.conf.json window entry gains `"decorations": false`.

- [ ] **Step 1: Failing test**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("../../lib/transport", async (orig) => ({
  ...(await orig()),
  isDesktop: () => true,
}));

import { WindowControls } from "../WindowControls";

describe("WindowControls", () => {
  it("renders minimize, maximize, close with accessible names", () => {
    render(<WindowControls />);
    expect(screen.getByRole("button", { name: "Minimize" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Maximize" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });
});
```

→ FAIL.

- [ ] **Step 2: Implement `WindowControls.tsx`**

```tsx
/**
 * Frameless-window controls. 44px hit targets, close turns recording-red on
 * hover — the one destructive-adjacent control announces itself.
 */
import { useEffect, useState } from "react";
import { Copy, Minus, Square, X } from "lucide-react";
import { isDesktop } from "../lib/transport";
import { cn } from "../lib/cn";

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isDesktop()) return;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      setMaximized(await win.isMaximized());
      unlisten = await win.onResized(async () => setMaximized(await win.isMaximized()));
    })();
    return () => unlisten?.();
  }, []);

  if (!isDesktop()) return null;

  async function call(action: "minimize" | "toggleMaximize" | "close") {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    if (action === "minimize") await win.minimize();
    else if (action === "toggleMaximize") await win.toggleMaximize();
    else await win.close(); // goes through CloseRequested → close-to-tray logic
  }

  const base =
    "inline-flex h-8 w-11 items-center justify-center text-fg-faint transition-colors hover:bg-hover hover:text-fg";
  return (
    <div className="flex items-center" data-testid="window-controls">
      <button type="button" aria-label="Minimize" className={base} onClick={() => void call("minimize")}>
        <Minus size={14} />
      </button>
      <button
        type="button"
        aria-label={maximized ? "Restore" : "Maximize"}
        className={base}
        onClick={() => void call("toggleMaximize")}
      >
        {maximized ? <Copy size={12} /> : <Square size={12} />}
      </button>
      <button
        type="button"
        aria-label="Close"
        className={cn(base, "hover:bg-recording hover:text-white")}
        onClick={() => void call("close")}
      >
        <X size={15} />
      </button>
    </div>
  );
}
```

→ PASS.

- [ ] **Step 3: The header becomes the titlebar** — in `App.tsx`, the
`<header …>` element gains `data-tauri-drag-region` and a double-click
handler, the wordmark, and the controls:

```tsx
        <header
          data-tauri-drag-region
          onDoubleClick={async (e) => {
            if (!isDesktop()) return;
            if ((e.target as HTMLElement).closest("button, input, select, a")) return;
            const { getCurrentWindow } = await import("@tauri-apps/api/window");
            await getCurrentWindow().toggleMaximize();
          }}
          className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-raised/80 py-1.5 pl-3 pr-0"
        >
          <RecordBar … />                                  {/* unchanged */}
          <span
            data-tauri-drag-region
            className="pointer-events-none hidden select-none text-[12px] font-semibold tracking-[0.08em] text-fg-faint sm:block"
          >
            NOTETAKER
          </span>
          <div className="flex items-center gap-1">
            <IconButton …theme toggle… />                   {/* unchanged */}
            <IconButton …settings… />                       {/* unchanged */}
            <WindowControls />
          </div>
        </header>
```

(Keep the two IconButtons exactly as they are; only the wrapper's padding
changes from `px-3 py-2` to the values above so Close sits flush in the
corner.)

- [ ] **Step 4: Frameless** — in `src-tauri/tauri.conf.json`, the window
entry becomes:

```json
      {
        "title": "Notetaker",
        "width": 1180,
        "height": 800,
        "minWidth": 380,
        "minHeight": 480,
        "decorations": false
      }
```

- [ ] **Step 5: Green + CI + Windows truth**

```bash
pnpm test --run && pnpm build
git add -A && git commit -m "feat(native): the window draws its own titlebar

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push && gh run watch $(gh run list --branch claude/app-ui-ux-overhaul-96e4c6 --limit 1 --json databaseId -q '.[0].databaseId') --exit-status
```

Then the manual titlebar checklist runs in Task 12's Windows pass: drag moves
the window; double-click header maximizes and restores; edge-drag snapping
works; minimize/close behave; close hides to tray.

---

### Task 11: The polish pass — glow where things are alive

**Files:**
- Modify: `src/components/ui.tsx` (primary button), `src/components/RecordBar.tsx`
  (meter fill), `src/styles/panels.css` (player thumb + first-run title)

**Interfaces:** none new — exact string replacements only.

- [ ] **Step 1: Primary buttons go aurora** — in `ui.tsx`, the `button` cva
variant line

```
        primary: "bg-accent text-accent-fg hover:bg-accent-hover",
```

becomes

```
        primary:
          "bg-[image:var(--grad-aurora)] text-accent-fg shadow-[var(--glow-accent)] hover:brightness-110",
```

- [ ] **Step 2: The meter glows** — in `RecordBar.tsx` line ~102, the class

```
          className="block h-full rounded-full bg-ok transition-[width] duration-50 ease-linear"
```

becomes

```
          className="block h-full rounded-full bg-[image:var(--grad-aurora)] shadow-[var(--glow-accent)] transition-[width] duration-50 ease-linear"
```

- [ ] **Step 3: Player thumb + first-run title** — append to
`src/styles/panels.css`:

```css
/* --- 2026-08-04 overhaul polish ---------------------------------------- */

/* The playhead thumb carries the accent glow — the one continuously-moving
   thing in playback should be the lit one. */
input[type="range"]::-webkit-slider-thumb {
  box-shadow: var(--glow-accent);
}

/* First-run greeting title in the aurora — the single place gradient text is
   allowed (spec §1). */
.first-run__header h2 {
  background: var(--grad-aurora);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
```

- [ ] **Step 4: Green + screenshots**

```bash
pnpm test --run && pnpm build
bash scripts/shoot-ui.sh /tmp/overhaul-shots/task11
```

Acceptance beats on both PNGs: primary buttons show the violet→cyan gradient;
nothing else gained a glow; light mode's "glow" reads as a soft tinted
shadow, not neon.

- [ ] **Step 5: Commit**

```bash
git add src/components/ui.tsx src/components/RecordBar.tsx src/styles/panels.css
git commit -m "style: glow lives only on live things

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 12: The sweep — MAP, gallery, Windows truth pass

**Files:**
- Modify: `docs/MAP.md`
- Create: `docs/superpowers/specs/assets/2026-08-04-pitch/after-light.png`, `after-dark.png`

- [ ] **Step 1: Full local verification**

```bash
pnpm test --run && pnpm build
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core && cargo clippy -p notetaker-core --all-targets -- -D warnings
cd .. && bash scripts/check-platforms.sh
```
All green, or stop and report.

- [ ] **Step 2: The after gallery**

```bash
bash scripts/shoot-ui.sh docs/superpowers/specs/assets/2026-08-04-pitch
mv docs/superpowers/specs/assets/2026-08-04-pitch/light.png docs/superpowers/specs/assets/2026-08-04-pitch/after-light.png
mv docs/superpowers/specs/assets/2026-08-04-pitch/dark.png docs/superpowers/specs/assets/2026-08-04-pitch/after-dark.png
```

Open both against `pitch-top.png` Plate 03. They must read as the same
design executed (rail + note + titlebar chrome; browser build shows no window
buttons — that difference is expected and correct).

- [ ] **Step 3: MAP update** — in `docs/MAP.md`, add to the top of the
State/current section (adapt to the section's existing format):

```markdown
- **UI overhaul "lit from within" (2026-08-04): COMPLETE on branch
  `claude/app-ui-ux-overhaul-96e4c6`.** Aurora token system (dark "luminous
  glass" / light "porcelain"), Echo icon + tray with recording state,
  close-to-tray, global hotkeys (Ctrl+Alt+N / Ctrl+Alt+Space, rebindable in
  Settings → Hotkeys), six-section Settings with mic picker and folder
  picker, library sort/filter, find-and-jump palette, custom titlebar.
  Spec: `docs/superpowers/specs/2026-08-04-ui-overhaul-design.md`. Before/
  after renders in `docs/superpowers/specs/assets/2026-08-04-pitch/`.
  Still owed from real hardware: the Windows interactive pass below.
```

And under next-work, add:

```markdown
- Windows truth pass for the overhaul: install the CI build, then check —
  tray icon states (idle/recording/paused) at 100% and 150% DPI, tray menu
  labels and Quit, close-to-tray + first-time "Still running" note, both
  global hotkeys with the window closed, titlebar drag / double-click /
  edge-snap, autostart after a reboot, mic picker lists real devices,
  PrintWindow screenshots of dark + light against the pitch.
```

- [ ] **Step 4: Commit + push + CI**

```bash
git add docs/MAP.md docs/superpowers/specs/assets/2026-08-04-pitch/after-*.png
git commit -m "docs: MAP carries the overhaul; before/after renders checked in

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push && gh run watch $(gh run list --branch claude/app-ui-ux-overhaul-96e4c6 --limit 1 --json databaseId -q '.[0].databaseId') --exit-status
```

- [ ] **Step 5: The Windows interactive pass** — from WSL, using the
`wsl-reaches-windows-host` technique (PowerShell on the host): download the
CI-built NSIS installer artifact (`gh run download <id> -n <installer-artifact-name>`),
install silently, launch, run the checklist from the MAP entry above, and
capture `PrintWindow` screenshots (flag 2) of the app in dark and light.
Anything that fails on real hardware gets a fix commit + re-run before the
branch is reported done. **Never take a full-screen grab.**

- [ ] **Step 6: Report** — final state to Mr. Brothers: tests/CI/clippy all
green, gallery committed, Windows checklist results item by item, and the
branch parked at **ready — say the word** (no merge, no release).
