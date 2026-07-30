import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/ipc";
import type { RecordingDetail, RecordingRow, SearchHit, Template } from "../lib/ipc";

/** The sidebar's fixed views plus "a task", per spec §4.4. */
export type LibraryView =
  | { kind: "all" }
  | { kind: "unsorted" }
  | { kind: "recent" }
  | { kind: "task"; name: string };

const SEARCH_DEBOUNCE_MS = 300;

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function sortByCreatedDesc(rows: RecordingRow[]): RecordingRow[] {
  // `created` is RFC3339, so lexicographic order tracks chronological order.
  return [...rows].sort((a, b) => b.created.localeCompare(a.created));
}

function filterByView(rows: RecordingRow[], view: LibraryView): RecordingRow[] {
  const sorted = sortByCreatedDesc(rows);
  switch (view.kind) {
    case "all":
      return sorted;
    case "unsorted":
      return sorted.filter((r) => r.task === null);
    case "recent":
      // The row shape has no "processed at" timestamp, so "recently
      // processed" is approximated as ready recordings, newest first.
      return sorted.filter((r) => r.status === "ready");
    case "task":
      return sorted.filter((r) => r.task === view.name);
  }
}

/**
 * Owns every call into `api` plus the fetched/derived state for the
 * library window. Components that consume this hook stay presentational.
 */
export function useLibrary() {
  const [tasks, setTasks] = useState<string[]>([]);
  const [recordings, setRecordings] = useState<RecordingRow[]>([]);
  const [view, setView] = useState<LibraryView>({ kind: "all" });
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
      setRecordings(await api.listRecordings());
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

  const visibleRecordings = useMemo(() => filterByView(recordings, view), [recordings, view]);

  return {
    tasks,
    recordings: visibleRecordings,
    view,
    setView,
    selectedId,
    selectRecording,
    detail,
    detailLoading,
    createTask,
    assignTask,
    renameRecording,
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
