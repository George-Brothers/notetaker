/**
 * Cmd+K — fast, local, full-text search.
 *
 * This is deliberately search-only. Actions belong in the visible toolbar;
 * the shortcut should answer one question reliably: "where did I put that?"
 */

import { useEffect, useRef, useState } from "react";
import { Command } from "cmdk";
import {
  FileText,
  Folder,
  MessageSquareText,
  NotebookPen,
  Search,
} from "lucide-react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { api, type SearchHit, type SearchHitKind } from "../lib/ipc";

const SEARCH_DEBOUNCE_MS = 180;

export function CommandPalette({
  open,
  onOpenChange,
  onSelectRecording,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelectRecording: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestRef = useRef(0);

  // Cmd+K / Ctrl+K from anywhere, including while a field has focus.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        onOpenChange(!open);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onOpenChange]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setResults([]);
    setError(null);
    setLoading(false);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const trimmed = query.trim();
    const request = ++requestRef.current;
    if (!trimmed) {
      setResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    const timer = window.setTimeout(() => {
      api
        .search(trimmed)
        .then((next) => {
          if (request !== requestRef.current) return;
          setResults(next);
          setError(null);
        })
        .catch((err: unknown) => {
          if (request !== requestRef.current) return;
          setError(err instanceof Error ? err.message : String(err));
          setResults([]);
        })
        .finally(() => {
          if (request === requestRef.current) setLoading(false);
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [open, query]);

  function select(id: string) {
    onOpenChange(false);
    onSelectRecording(id);
  }

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-black/35 backdrop-blur-[2px]" />
        <DialogPrimitive.Content
          aria-label="Search local content"
          className="fixed left-1/2 top-[15vh] z-50 w-[calc(100vw-2rem)] max-w-xl -translate-x-1/2 overflow-hidden rounded-[var(--radius-card)] border border-border bg-raised shadow-[var(--shadow-pop)]"
        >
          <DialogPrimitive.Title className="sr-only">Search local content</DialogPrimitive.Title>
          <Command loop shouldFilter={false} className="flex flex-col">
            <div className="flex items-center border-b border-border px-4">
              <Search size={15} className="shrink-0 text-fg-faint" aria-hidden />
              <Command.Input
                autoFocus
                value={query}
                onValueChange={setQuery}
                placeholder="Search meetings, notes, transcripts…"
                className="h-12 w-full bg-transparent px-2 text-[15px] text-fg placeholder:text-fg-faint focus:outline-none"
              />
            </div>
            <Command.List className="max-h-[min(28rem,60vh)] overflow-y-auto p-1.5">
              {loading && (
                <Command.Loading className="px-3 py-3 text-[13px] text-fg-muted">
                  Searching your local library…
                </Command.Loading>
              )}
              {!loading && error && (
                <p role="alert" className="px-3 py-4 text-center text-[13px] text-error">
                  Search failed: {error}
                </p>
              )}
              {!loading && !error && query.trim() && results.length === 0 && (
                <Command.Empty className="px-3 py-6 text-center text-[13px] text-fg-muted">
                  No local matches.
                </Command.Empty>
              )}
              {!query.trim() && (
                <p className="px-3 py-6 text-center text-[13px] text-fg-muted">
                  Search titles, folders, transcripts, summaries, and your notes.
                </p>
              )}
              {results.length > 0 && (
                <Command.Group
                  heading="Local library"
                  className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-fg-faint"
                >
                  {results.map((hit) => (
                    <Command.Item
                      key={hit.id}
                      value={`${hit.id} ${hit.title} ${hit.task ?? "Unsorted"} ${hit.snippet}`}
                      onSelect={() => select(hit.id)}
                      className="flex cursor-pointer items-start gap-2.5 rounded-[var(--radius-control)] px-2 py-2 text-[14px] text-fg data-[selected=true]:bg-selected"
                    >
                      <span className="mt-0.5 shrink-0 text-fg-faint">
                        <HitIcon kind={hit.kind} />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="flex items-baseline gap-2">
                          <span className="min-w-0 flex-1 truncate font-medium">{hit.title}</span>
                          <span className="shrink-0 text-[11px] text-fg-faint">
                            {kindLabel(hit.kind)}
                          </span>
                        </span>
                        <span className="block truncate text-[11px] text-fg-faint">
                          {hit.task ?? "Unsorted"}
                        </span>
                        {hit.snippet && (
                          <span className="mt-0.5 block line-clamp-2 text-[12px] leading-snug text-fg-muted">
                            {highlightSnippet(hit.snippet)}
                          </span>
                        )}
                      </span>
                    </Command.Item>
                  ))}
                </Command.Group>
              )}
            </Command.List>
          </Command>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function HitIcon({ kind }: { kind: SearchHitKind }) {
  switch (kind) {
    case "title":
      return <FileText size={14} />;
    case "folder":
      return <Folder size={14} />;
    case "transcript":
      return <MessageSquareText size={14} />;
    case "summary":
      return <FileText size={14} />;
    case "notes":
      return <NotebookPen size={14} />;
  }
}

function kindLabel(kind: SearchHitKind): string {
  switch (kind) {
    case "title":
      return "Title";
    case "folder":
      return "Folder";
    case "transcript":
      return "Transcript";
    case "summary":
      return "Summary";
    case "notes":
      return "Notes";
  }
}

function highlightSnippet(snippet: string) {
  return snippet.split(/(<b>[\s\S]*?<\/b>)/gi).map((part, index) => {
    const match = part.match(/^<b>([\s\S]*)<\/b>$/i);
    return match ? (
      <mark key={index} className="rounded bg-accent-soft px-0.5 text-fg">
        {match[1]}
      </mark>
    ) : (
      <span key={index}>{part}</span>
    );
  });
}
