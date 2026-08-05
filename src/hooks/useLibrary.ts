import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/ipc";
import type { RecordingDetail, RecordingRow, SearchHit, Template } from "../lib/ipc";

/** The sidebar's fixed views plus "a task", per spec §4.4. */
export type LibraryView =
  | { kind: "all" }
  | { kind: "unsorted" }
  | { kind: "recent" }
  | { kind: "archive" }
  | { kind: "task"; name: string };

const SEARCH_DEBOUNCE_MS = 300;

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function filterByView(rows: RecordingRow[], view: LibraryView): RecordingRow[] {
  const sorted = rows;
  switch (view.kind) {
    case "all":
      return sorted;
    case "unsorted":
      return sorted.filter((r) => r.task === null);
    case "recent":
      // The row shape has no "processed at" timestamp, so "recently
      // processed" is approximated as ready recordings, newest first.
      return sorted.filter((r) => r.status === "ready");
    case "archive":
      return sorted;
    case "task":
      return sorted.filter((r) => r.task === view.name);
  }
}

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

/**
 * Owns every call into `api` plus the fetched/derived state for the
 * library window. Components that consume this hook stay presentational.
 */
export function useLibrary() {
  const [tasks, setTasks] = useState<string[]>([]);
  const [recordings, setRecordings] = useState<RecordingRow[]>([]);
  const [archivedRecordings, setArchivedRecordings] = useState<RecordingRow[]>([]);
  const [view, setView] = useState<LibraryView>({ kind: "all" });
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<RecordingDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [searchResults, setSearchResults] = useState<SearchHit[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refreshTasks = useCallback(async () => {
    try {
      setTasks(await api.listTasks());
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

  const refreshRecordings = useCallback(async () => {
    try {
      const [active, archived] = await Promise.all([api.listRecordings(), api.listArchivedRecordings()]);
      setRecordings(active);
      setArchivedRecordings(archived);
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

  useEffect(() => {
    refreshTasks();
    refreshRecordings();
  }, [refreshTasks, refreshRecordings]);

  const selectRecording = useCallback(async (id: string) => {
    setSelectedId(id);
    setDetailLoading(true);
    try {
      setDetail(await api.getRecording(id));
    } catch (err) {
      setLoadError(describeError(err));
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const createTask = useCallback(
    async (name: string) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      try {
        await api.createTask(trimmed);
        await refreshTasks();
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [refreshTasks]
  );

  const assignTask = useCallback(
    async (id: string, task: string) => {
      try {
        await api.assignTask(id, task);
        await refreshRecordings();
        if (selectedId === id) {
          setDetail(await api.getRecording(id));
        }
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [refreshRecordings, selectedId]
  );

  const renameRecording = useCallback(
    async (id: string, title: string) => {
      const trimmed = title.trim();
      if (!trimmed) return;
      try {
        await api.renameRecording(id, trimmed);
        // The title lives in the on-disk folder name, so a rename moves the
        // recording; refetch both the list and the open detail rather than
        // patching the old title in place.
        await refreshRecordings();
        if (selectedId === id) {
          setDetail(await api.getRecording(id));
        }
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [refreshRecordings, selectedId]
  );

  const archiveRecording = useCallback(
    async (id: string) => {
      try {
        await api.archiveRecording(id);
        await refreshRecordings();
        if (selectedId === id) {
          setSelectedId(null);
          setDetail(null);
        }
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [refreshRecordings, selectedId]
  );

  const restoreRecording = useCallback(
    async (id: string) => {
      try {
        await api.restoreRecording(id);
        await refreshRecordings();
        if (selectedId === id) {
          setSelectedId(null);
          setDetail(null);
        }
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [refreshRecordings, selectedId]
  );

  const deleteRecording = useCallback(
    async (id: string) => {
      try {
        await api.deleteRecording(id);
        await refreshRecordings();
        if (selectedId === id) {
          setSelectedId(null);
          setDetail(null);
        }
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [refreshRecordings, selectedId]
  );

  const renameSpeaker = useCallback(
    async (id: string, key: string, name: string) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      try {
        await api.renameSpeaker(id, key, trimmed);
        if (selectedId === id) {
          setDetail(await api.getRecording(id));
        }
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [selectedId]
  );

  const saveSummary = useCallback(
    async (id: string, summaryMd: string) => {
      try {
        await api.updateSummary(id, summaryMd);
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    []
  );

  // --- the notepad ------------------------------------------------------

  /**
   * Saves the user's typed notes.
   *
   * Deliberately does *not* refetch the detail afterwards. The user is very
   * likely still typing, and replacing the textarea's value from the server
   * mid-keystroke is how autosave eats a word. The notepad owns its own text;
   * this only persists it.
   */
  const saveNotes = useCallback(async (id: string, notesMd: string) => {
    try {
      await api.saveNotes(id, notesMd);
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

  const [templates, setTemplates] = useState<Template[]>([]);
  useEffect(() => {
    api
      .listTemplates()
      .then(setTemplates)
      // A picker with nothing in it is a small problem; a shell that fails to
      // load over it is a large one.
      .catch((err) => setLoadError(describeError(err)));
  }, []);

  const setTemplate = useCallback(
    async (id: string, template: string) => {
      try {
        await api.setTemplate(id, template);
        if (selectedId === id) setDetail(await api.getRecording(id));
      } catch (err) {
        setLoadError(describeError(err));
      }
    },
    [selectedId]
  );

  /**
   * Ticks or unticks one action item.
   *
   * The command returns the whole re-parsed checklist rather than a success
   * flag, because ticking rewrites `summary.md` and the indices shift if the
   * summary was edited elsewhere. Patching the list from the response is the
   * only version that cannot tick the wrong box on the next click.
   */
  const toggleAction = useCallback(async (id: string, index: number, done: boolean) => {
    try {
      const actions = await api.setActionDone(id, index, done);
      setDetail((current) =>
        current && current.id === id ? { ...current, actions } : current
      );
    } catch (err) {
      setLoadError(describeError(err));
    }
  }, []);

  /** Accepts the AI's suggested title, which is just a rename. */
  const acceptSuggestedTitle = useCallback(
    async (id: string, title: string) => {
      await renameRecording(id, title);
    },
    [renameRecording]
  );

  const dismissError = useCallback(() => setLoadError(null), []);

  /**
   * Closes the open recording.
   *
   * Only the narrow layout uses this: on a phone the rail and the note are two
   * screens rather than two panes, and "nothing selected" is what shows the
   * list again.
   */
  const clearSelection = useCallback(() => {
    setSelectedId(null);
    setDetail(null);
  }, []);

  const search = useCallback((q: string) => {
    setQuery(q);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!q.trim()) {
      setSearchResults(null);
      return;
    }
    debounceRef.current = setTimeout(async () => {
      try {
        setSearchResults(await api.search(q));
      } catch (err) {
        setLoadError(describeError(err));
      }
    }, SEARCH_DEBOUNCE_MS);
  }, []);

  useEffect(
    () => () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    },
    []
  );

  const source = view.kind === "archive" ? archivedRecordings : recordings;
  const visibleRecordings = useMemo(
    () => applySort(applyFilter(filterByView(source, view), filter), sort),
    [source, view, filter, sort],
  );

  return {
    tasks,
    recordings: visibleRecordings,
    view,
    setView,
    sort,
    setSort,
    filter,
    setFilter,
    selectedId,
    selectRecording,
    clearSelection,
    detail,
    detailLoading,
    createTask,
    assignTask,
    renameRecording,
    archiveRecording,
    restoreRecording,
    deleteRecording,
    renameSpeaker,
    saveSummary,
    saveNotes,
    templates,
    setTemplate,
    toggleAction,
    acceptSuggestedTitle,
    refreshRecordings,
    query,
    search,
    searchResults,
    loadError,
    dismissError,
  };
}
