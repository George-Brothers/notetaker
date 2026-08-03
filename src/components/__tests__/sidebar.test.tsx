import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { Sidebar } from "../Sidebar";
import type { RecordingRow } from "../../lib/ipc";

const queued: RecordingRow = {
  id: "queued-recording",
  title: "Lecture",
  task: null,
  created: "2026-08-01T12:00:00Z",
  durationS: 60,
  mode: "in_person",
  status: "queued",
  suggestedTask: null,
  suggestedTitle: null,
  error: null,
  captureNote: null,
  hasNotes: false,
};

afterEach(cleanup);

describe("Sidebar setup status", () => {
  it("tells a queued recording why it is waiting when speech models are missing", () => {
    render(
      <Sidebar
        tasks={[]}
        activeView={{ kind: "all" }}
        onSelectView={vi.fn()}
        onCreateTask={vi.fn()}
        recordings={[queued]}
        selectedId={null}
        onSelectRecording={vi.fn()}
        query=""
        onSearch={vi.fn()}
        searchResults={null}
        onOpenPalette={vi.fn()}
        modelsMissing
      />,
    );

    expect(screen.getByText(/waiting on the speech models/i)).toBeInTheDocument();
  });

  it("renames a folder from its inline folder controls", () => {
    const onRenameTask = vi.fn();
    render(
      <Sidebar
        tasks={["Finance"]}
        activeView={{ kind: "all" }}
        onSelectView={vi.fn()}
        onCreateTask={vi.fn()}
        onRenameTask={onRenameTask}
        onDeleteTask={vi.fn()}
        recordings={[]}
        selectedId={null}
        onSelectRecording={vi.fn()}
        query=""
        onSearch={vi.fn()}
        searchResults={null}
        onOpenPalette={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Rename folder Finance" }));
    fireEvent.change(screen.getByLabelText("Rename folder Finance"), {
      target: { value: "Planning" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save folder Finance" }));

    expect(onRenameTask).toHaveBeenCalledWith("Finance", "Planning");
  });

  it("requires confirmation before moving a deleted folder to Unsorted", () => {
    const onDeleteTask = vi.fn();
    render(
      <Sidebar
        tasks={["Finance"]}
        activeView={{ kind: "all" }}
        onSelectView={vi.fn()}
        onCreateTask={vi.fn()}
        onRenameTask={vi.fn()}
        onDeleteTask={onDeleteTask}
        recordings={[]}
        selectedId={null}
        onSelectRecording={vi.fn()}
        query=""
        onSearch={vi.fn()}
        searchResults={null}
        onOpenPalette={vi.fn()}
      />,
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Delete folder Finance" })[0]);
    expect(screen.getByRole("alert")).toHaveTextContent(/move to Unsorted/i);
    expect(onDeleteTask).not.toHaveBeenCalled();
    fireEvent.click(within(screen.getByRole("alert")).getByRole("button", { name: "Delete folder" }));
    expect(onDeleteTask).toHaveBeenCalledWith("Finance");
  });

  it("selects multiple recordings and exposes folder, archive, and delete actions", () => {
    const second: RecordingRow = {
      ...queued,
      id: "ready-recording",
      title: "Budget sync",
      status: "ready",
    };
    const onMoveSelected = vi.fn();
    const onArchiveSelected = vi.fn();
    const onDeleteSelected = vi.fn();

    function Harness() {
      const [selectedIds, setSelectedIds] = useState<string[]>([]);
      return (
        <Sidebar
          tasks={["Finance"]}
          activeView={{ kind: "all" }}
          onSelectView={vi.fn()}
          onCreateTask={vi.fn()}
          recordings={[queued, second]}
          selectedId={null}
          selectedIds={selectedIds}
          onSelectRecording={vi.fn()}
          onToggleRecordingSelection={(id) =>
            setSelectedIds((current) =>
              current.includes(id) ? current.filter((value) => value !== id) : [...current, id],
            )
          }
          onMoveSelected={onMoveSelected}
          onArchiveSelected={onArchiveSelected}
          onDeleteSelected={onDeleteSelected}
          query=""
          onSearch={vi.fn()}
          searchResults={null}
          onOpenPalette={vi.fn()}
        />
      );
    }

    render(<Harness />);

    fireEvent.click(screen.getByRole("checkbox", { name: "Select Lecture" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Budget sync" }));

    const bulk = screen.getByRole("region", { name: "Bulk actions" });
    expect(bulk).toHaveTextContent("2 selected");
    fireEvent.change(screen.getByRole("combobox", { name: "Move selected recordings to folder" }), {
      target: { value: "Finance" },
    });
    expect(onMoveSelected).toHaveBeenCalledWith("Finance");

    fireEvent.click(within(bulk).getByRole("button", { name: "Archive" }));
    expect(onArchiveSelected).toHaveBeenCalled();

    fireEvent.click(within(bulk).getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("alert").getElementsByTagName("button")[0]);
    expect(onDeleteSelected).toHaveBeenCalled();
  });
});
