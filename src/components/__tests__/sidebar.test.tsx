import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
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
});
