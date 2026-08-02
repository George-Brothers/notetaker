import { fireEvent, render, screen, waitFor, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { CommandPalette } from "../CommandPalette";
import { api } from "../../lib/ipc";
import type { SearchHit } from "../../lib/ipc";

vi.mock("../../lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/ipc")>();
  return {
    ...actual,
    api: Object.fromEntries(Object.keys(actual.api).map((key) => [key, vi.fn()])),
  };
});

const hit: SearchHit = {
  id: "meeting-1",
  title: "Budget sync",
  task: "Finance",
  snippet: "Discussed <b>budget</b> timing in the transcript.",
  kind: "transcript",
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.mocked(api.search).mockResolvedValue([hit]);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("CommandPalette", () => {
  it("debounces local full-text search and highlights the matching field", async () => {
    render(<CommandPalette open onOpenChange={vi.fn()} onSelectRecording={vi.fn()} />);

    fireEvent.change(screen.getByPlaceholderText(/Search meetings/i), {
      target: { value: "budget" },
    });

    await waitFor(() => expect(api.search).toHaveBeenCalledWith("budget"));
    expect(screen.getByText("Budget sync")).toBeInTheDocument();
    expect(screen.getByText("Transcript")).toBeInTheDocument();
    expect(screen.getByText("budget").tagName).toBe("MARK");
  });

  it("opens and closes from Ctrl+K without exposing action commands", () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return <CommandPalette open={open} onOpenChange={setOpen} onSelectRecording={vi.fn()} />;
    }

    render(<Harness />);
    fireEvent.keyDown(document, { key: "k", ctrlKey: true });
    expect(screen.getByRole("dialog", { name: "Search local content" })).toBeInTheDocument();
    expect(screen.queryByText("Record a meeting — both sides of the call")).not.toBeInTheDocument();
  });
});
