import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
  ReactNode,
} from "react";
import { cn } from "../lib/cn";

export const DESKTOP_LAYOUT_BREAKPOINT = 1000;
export const SINGLE_PANE_BREAKPOINT = 720;
export const PRIMARY_PANE_MIN = 480;
export const LIBRARY_PANE_MIN = 220;
export const LIBRARY_PANE_MAX = 360;
export const ASK_PANE_MIN = 300;
export const ASK_PANE_MAX = 480;
export const PANE_KEYBOARD_STEP = 16;

export const DEFAULT_PANE_SIZES = {
  library: 264,
  ask: 340,
} as const;

export interface PaneSizes {
  library: number;
  ask: number;
}

export type DesktopLayoutMode = "desktop" | "ask-overlay" | "single-pane";
export type MobilePane = "library" | "primary";

export function clampPaneWidth(
  value: number | null | undefined,
  kind: "library" | "ask",
): number {
  const fallback = kind === "library" ? DEFAULT_PANE_SIZES.library : DEFAULT_PANE_SIZES.ask;
  const min = kind === "library" ? LIBRARY_PANE_MIN : ASK_PANE_MIN;
  const max = kind === "library" ? LIBRARY_PANE_MAX : ASK_PANE_MAX;
  const numeric = Number.isFinite(value) ? Number(value) : fallback;
  return Math.round(Math.max(min, Math.min(max, numeric)));
}

export function clampPaneSizes(value: Partial<PaneSizes> | null | undefined): PaneSizes {
  return {
    library: clampPaneWidth(value?.library, "library"),
    ask: clampPaneWidth(value?.ask, "ask"),
  };
}

export function layoutModeForWidth(width: number): DesktopLayoutMode {
  if (width < SINGLE_PANE_BREAKPOINT) return "single-pane";
  if (width < DESKTOP_LAYOUT_BREAKPOINT) return "ask-overlay";
  return "desktop";
}

function shrinkRailsToFit(library: number, ask: number, available: number): PaneSizes {
  let nextLibrary = library;
  let nextAsk = ask;
  let deficit = Math.max(0, nextLibrary + nextAsk - available);

  while (deficit > 0) {
    const libraryRoom = Math.max(0, nextLibrary - LIBRARY_PANE_MIN);
    const askRoom = Math.max(0, nextAsk - ASK_PANE_MIN);
    const room = libraryRoom + askRoom;
    if (room === 0) break;

    const libraryCut = Math.min(libraryRoom, Math.round((deficit * libraryRoom) / room));
    const askCut = Math.min(askRoom, deficit - libraryCut);
    nextLibrary -= libraryCut;
    nextAsk -= askCut;
    const removed = libraryCut + askCut;
    if (removed === 0) break;
    deficit -= removed;
  }

  return { library: nextLibrary, ask: nextAsk };
}

/**
 * Returns the widths that can actually fit while retaining the primary pane's
 * minimum. The saved values remain the user's preference; these values are
 * only a constrained rendering calculation for the current window size.
 */
export function effectivePaneSizes(
  viewportWidth: number,
  requested: PaneSizes,
  askOpen: boolean,
): PaneSizes {
  const clamped = clampPaneSizes(requested);
  const mode = layoutModeForWidth(viewportWidth);
  if (mode === "single-pane") return clamped;

  const libraryRoom = Math.max(LIBRARY_PANE_MIN, viewportWidth - PRIMARY_PANE_MIN);
  const library = Math.min(clamped.library, libraryRoom);
  if (mode !== "desktop" || !askOpen) return { library, ask: clamped.ask };

  const availableRails = Math.max(
    LIBRARY_PANE_MIN + ASK_PANE_MIN,
    viewportWidth - PRIMARY_PANE_MIN,
  );
  return shrinkRailsToFit(clamped.library, clamped.ask, availableRails);
}

function focusableElements(container: HTMLElement): HTMLElement[] {
  return Array.from(
    container.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((element) => !element.hidden && element.getAttribute("aria-hidden") !== "true");
}

interface PaneSeparatorProps {
  id: string;
  label: string;
  controls: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  onCommit: () => void;
  kind: "library" | "ask";
}

function PaneSeparator({
  id,
  label,
  controls,
  value,
  min,
  max,
  onChange,
  onCommit,
  kind,
}: PaneSeparatorProps) {
  const pointerStart = useRef<{ pointerId: number; x: number; value: number } | null>(null);

  function setFromKeyboard(next: number) {
    onChange(Math.max(min, Math.min(max, next)));
    onCommit();
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    let next: number | null = null;
    if (event.key === "ArrowLeft") next = value - PANE_KEYBOARD_STEP;
    else if (event.key === "ArrowRight") next = value + PANE_KEYBOARD_STEP;
    else if (event.key === "PageDown") next = value - PANE_KEYBOARD_STEP * 2;
    else if (event.key === "PageUp") next = value + PANE_KEYBOARD_STEP * 2;
    else if (event.key === "Home") next = min;
    else if (event.key === "End") next = max;
    if (next === null) return;
    event.preventDefault();
    setFromKeyboard(next);
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    event.preventDefault();
    pointerStart.current = { pointerId: event.pointerId, x: event.clientX, value };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    const start = pointerStart.current;
    if (!start || start.pointerId !== event.pointerId) return;
    const delta = event.clientX - start.x;
    const next = kind === "library" ? start.value + delta : start.value - delta;
    onChange(Math.max(min, Math.min(max, next)));
  }

  function onPointerUp(event: ReactPointerEvent<HTMLDivElement>) {
    if (!pointerStart.current || pointerStart.current.pointerId !== event.pointerId) return;
    pointerStart.current = null;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    onCommit();
  }

  return (
    <div
      id={id}
      role="separator"
      aria-label={label}
      aria-orientation="vertical"
      aria-controls={controls}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-valuetext={`${value} pixels`}
      aria-describedby={`${id}-instructions`}
      aria-keyshortcuts="ArrowLeft ArrowRight PageUp PageDown Home End"
      data-keyboard-step={PANE_KEYBOARD_STEP}
      tabIndex={0}
      className="desktop-shell__separator"
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <span className="desktop-shell__separator-handle" aria-hidden="true" />
      <span id={`${id}-instructions`} className="sr-only">
        Use Left and Right Arrow keys to resize in {PANE_KEYBOARD_STEP} pixel increments. Page Up
        and Page Down resize by two increments. Home and End set the minimum and maximum.
      </span>
    </div>
  );
}

export interface DesktopShellProps {
  library: ReactNode;
  primary: ReactNode;
  ask?: ReactNode;
  askOpen: boolean;
  onAskOpenChange: (open: boolean) => void;
  mobilePane: MobilePane;
  initialLibraryWidth?: number;
  initialAskWidth?: number;
  onPaneSizesCommit?: (sizes: PaneSizes) => void;
  className?: string;
}

export function DesktopShell({
  library,
  primary,
  ask,
  askOpen,
  onAskOpenChange,
  mobilePane,
  initialLibraryWidth,
  initialAskWidth,
  onPaneSizesCommit,
  className,
}: DesktopShellProps) {
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window === "undefined" ? 1440 : window.innerWidth,
  );
  const [requestedSizes, setRequestedSizes] = useState<PaneSizes>(() =>
    clampPaneSizes({ library: initialLibraryWidth, ask: initialAskWidth }),
  );
  const requestedSizesRef = useRef(requestedSizes);
  requestedSizesRef.current = requestedSizes;
  const restoreSignature = `${initialLibraryWidth ?? ""}:${initialAskWidth ?? ""}`;
  const restoredSignature = useRef<string | null>(null);

  useEffect(() => {
    const onResize = () => setViewportWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (initialLibraryWidth === undefined && initialAskWidth === undefined) return;
    if (restoredSignature.current === restoreSignature) return;
    restoredSignature.current = restoreSignature;
    const raw = { library: initialLibraryWidth, ask: initialAskWidth };
    const next = clampPaneSizes(raw);
    requestedSizesRef.current = next;
    setRequestedSizes(next);
    if (
      onPaneSizesCommit &&
      (raw.library !== undefined && raw.library !== next.library ||
        raw.ask !== undefined && raw.ask !== next.ask)
    ) {
      onPaneSizesCommit(next);
    }
  }, [initialAskWidth, initialLibraryWidth, onPaneSizesCommit, restoreSignature]);

  const updatePaneSize = useCallback((kind: "library" | "ask", value: number) => {
    const next = {
      ...requestedSizesRef.current,
      [kind]: clampPaneWidth(value, kind),
    };
    requestedSizesRef.current = next;
    setRequestedSizes(next);
  }, []);

  const commitPaneSizes = useCallback(() => {
    onPaneSizesCommit?.({ ...requestedSizesRef.current });
  }, [onPaneSizesCommit]);

  const mode = layoutModeForWidth(viewportWidth);
  const effective = effectivePaneSizes(viewportWidth, requestedSizes, askOpen);
  const style = {
    "--library-pane": `${effective.library}px`,
    "--ask-pane": `${effective.ask}px`,
  } as CSSProperties;
  const askIsDrawer = mode !== "desktop";
  const askRef = useRef<HTMLElement | null>(null);
  const previousFocus = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!askOpen) return;
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!askIsDrawer) return;
    const pane = askRef.current;
    if (!pane) return;
    const initial = pane.querySelector<HTMLElement>("[data-ask-initial-focus]");
    (initial ?? focusableElements(pane)[0])?.focus();
    return () => {
      const target = previousFocus.current;
      if (target && document.contains(target)) target.focus();
    };
  }, [askIsDrawer, askOpen]);

  function onAskKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onAskOpenChange(false);
      return;
    }
    if (!askIsDrawer || event.key !== "Tab") return;
    const pane = askRef.current;
    if (!pane) return;
    const focusables = focusableElements(pane);
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  const rootClassName = cn("desktop-shell", className);
  return (
    <div
      className={rootClassName}
      style={style}
      data-testid="desktop-shell"
      data-layout-mode={mode}
      data-ask-open={askOpen ? "true" : "false"}
      data-mobile-pane={mobilePane}
    >
      <section
        className="desktop-shell__library"
        id="library-pane"
        aria-label="Library"
        hidden={mode === "single-pane" && mobilePane === "primary"}
        aria-hidden={mode === "single-pane" && mobilePane === "primary" ? true : undefined}
      >
        {library}
        {mode === "desktop" && (
          <PaneSeparator
            id="library-pane-separator"
            label="Resize library pane"
            controls="library-pane"
            kind="library"
            value={effective.library}
            min={LIBRARY_PANE_MIN}
            max={LIBRARY_PANE_MAX}
            onChange={(value) => updatePaneSize("library", value)}
            onCommit={commitPaneSizes}
          />
        )}
      </section>

      <section
        className="desktop-shell__primary"
        id="primary-pane"
        aria-label="Recording"
        hidden={mode === "single-pane" && mobilePane === "library"}
        aria-hidden={mode === "single-pane" && mobilePane === "library" ? true : undefined}
      >
        {primary}
      </section>

      {askOpen && (
        <>
          {askIsDrawer && (
            <button
              type="button"
              className="desktop-shell__scrim"
              aria-label="Close Ask panel"
              onClick={() => onAskOpenChange(false)}
            />
          )}
          <aside
            ref={askRef}
            id="ask-pane"
            className="desktop-shell__ask"
            aria-label="Ask about this recording"
            role={askIsDrawer ? "dialog" : "region"}
            aria-modal={askIsDrawer ? true : undefined}
            onKeyDown={onAskKeyDown}
          >
            {ask}
            {mode === "desktop" && (
              <PaneSeparator
                id="ask-pane-separator"
                label="Resize Ask pane"
                controls="ask-pane"
                kind="ask"
                value={effective.ask}
                min={ASK_PANE_MIN}
                max={ASK_PANE_MAX}
                onChange={(value) => updatePaneSize("ask", value)}
                onCommit={commitPaneSizes}
              />
            )}
          </aside>
        </>
      )}
    </div>
  );
}
