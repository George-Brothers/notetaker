import { useMemo } from "react";
import { useLibrary } from "./hooks/useLibrary";
import type { LibraryView } from "./hooks/useLibrary";
import { Sidebar } from "./components/Sidebar";
import { RecordingList } from "./components/RecordingList";
import { RecordingDetail } from "./components/RecordingDetail";
import { SearchBar } from "./components/SearchBar";
import type { SearchHit } from "./lib/ipc";
import "./App.css";

function viewTitle(view: LibraryView): string {
  switch (view.kind) {
    case "all":
      return "All recordings";
    case "unsorted":
      return "Unsorted";
    case "recent":
      return "Recently processed";
    case "task":
      return view.name;
  }
}

function SearchResults({ hits, onSelect }: { hits: SearchHit[]; onSelect: (id: string) => void }) {
  if (hits.length === 0) {
    return <p className="empty-state">No matches. Try a different word or phrase.</p>;
  }
  return (
    <ul className="search-results" aria-label="Search results">
      {hits.map((hit) => (
        <li key={hit.id}>
          <button type="button" className="search-result" onClick={() => onSelect(hit.id)}>
            <span className="search-result__title">{hit.title}</span>
            <span className="search-result__task">{hit.task ?? "Unsorted"}</span>
            <span className="search-result__snippet">{hit.snippet}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function App() {
  const lib = useLibrary();
  const isSearching = useMemo(() => lib.query.trim().length > 0, [lib.query]);

  return (
    <div className="app-shell">
      <Sidebar tasks={lib.tasks} activeView={lib.view} onSelectView={lib.setView} onCreateTask={lib.createTask} />

      <div className="library-pane">
        <SearchBar query={lib.query} onSearch={lib.search} />

        {isSearching ? (
          <SearchResults hits={lib.searchResults ?? []} onSelect={lib.selectRecording} />
        ) : (
          <>
            <h2 className="library-pane__title">{viewTitle(lib.view)}</h2>
            <RecordingList
              recordings={lib.recordings}
              tasks={lib.tasks}
              selectedId={lib.selectedId}
              onSelect={lib.selectRecording}
              onAssignTask={lib.assignTask}
            />
          </>
        )}
      </div>

      <RecordingDetail
        detail={lib.detail}
        loading={lib.detailLoading}
        onRenameSpeaker={lib.renameSpeaker}
        onSaveSummary={lib.saveSummary}
      />
    </div>
  );
}

export default App;
