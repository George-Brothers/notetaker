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
