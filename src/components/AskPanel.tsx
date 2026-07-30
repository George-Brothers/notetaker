/**
 * "Ask this meeting" — Granola's Cmd+J, answered by the local model.
 *
 * The exchange is kept in this component and thrown away when the recording
 * changes, because the backend is deliberately single-turn: every question
 * re-sends the whole recording and carries no history. Showing a scrollback
 * that the model cannot actually see would be a lie about what it knows, so the
 * history here is a record of what *you* asked, not context for the next
 * answer — and the placeholder says so.
 */

import { useEffect, useRef, useState } from "react";
import { CornerDownLeft, Loader2, Sparkles } from "lucide-react";
import { api } from "../lib/ipc";
import { Button, Kbd, Notice, modKey } from "./ui";

interface Exchange {
  question: string;
  answer: string | null;
  error: string | null;
}

const SUGGESTIONS = [
  "What did we decide?",
  "What am I on the hook for?",
  "What numbers were mentioned?",
];

export function AskPanel({
  recordingId,
  canAsk,
}: {
  recordingId: string;
  canAsk: boolean;
}) {
  const [question, setQuestion] = useState("");
  const [history, setHistory] = useState<Exchange[]>([]);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const endRef = useRef<HTMLDivElement | null>(null);

  // A different recording is a different conversation.
  useEffect(() => {
    setHistory([]);
    setQuestion("");
  }, [recordingId]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end", behavior: "smooth" });
  }, [history]);

  async function ask(text: string) {
    const trimmed = text.trim();
    if (!trimmed || busy) return;
    setQuestion("");
    setBusy(true);
    const at = history.length;
    setHistory((h) => [...h, { question: trimmed, answer: null, error: null }]);
    try {
      const answer = await api.askRecording(recordingId, trimmed);
      setHistory((h) => h.map((e, i) => (i === at ? { ...e, answer } : e)));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setHistory((h) => h.map((e, i) => (i === at ? { ...e, error: message } : e)));
    } finally {
      setBusy(false);
      inputRef.current?.focus();
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        {history.length === 0 ? (
          <div className="flex flex-col gap-3 py-2">
            <p className="text-[13px] leading-relaxed text-fg-muted">
              Ask anything about this recording. It is answered on this machine, from this
              recording's own notes and transcript — nothing is sent anywhere.
            </p>
            <div className="flex flex-col items-start gap-1.5">
              {SUGGESTIONS.map((s) => (
                <button
                  key={s}
                  type="button"
                  disabled={!canAsk}
                  onClick={() => ask(s)}
                  className="rounded-full border border-border px-2.5 py-1 text-[13px] text-fg-muted transition-colors hover:bg-hover hover:text-fg disabled:opacity-45"
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
        ) : (
          <ul className="flex flex-col gap-4 py-2">
            {history.map((e, i) => (
              <li key={i} className="flex flex-col gap-1.5">
                <p className="text-[14px] font-medium text-fg">{e.question}</p>
                {e.answer !== null && (
                  <p className="flex gap-2 text-[14px] leading-relaxed text-fg-ai">
                    <Sparkles size={13} className="mt-1 shrink-0 text-accent" aria-hidden />
                    <span>{e.answer}</span>
                  </p>
                )}
                {e.error !== null && <Notice>{e.error}</Notice>}
                {e.answer === null && e.error === null && (
                  <p className="flex items-center gap-2 text-[13px] text-fg-muted">
                    <Loader2 size={13} className="animate-spin" aria-hidden />
                    Thinking…
                  </p>
                )}
              </li>
            ))}
            <div ref={endRef} />
          </ul>
        )}
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          void ask(question);
        }}
        className="mt-2 flex items-center gap-2 border-t border-border pt-3"
      >
        <input
          ref={inputRef}
          autoFocus
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          disabled={!canAsk || busy}
          placeholder={
            canAsk ? "Ask about this recording…" : "Available once this recording is processed"
          }
          aria-label="Your question about this recording"
          className="h-9 min-w-0 flex-1 rounded-[var(--radius-control)] border border-border bg-sunken px-2.5 text-[14px] text-fg placeholder:text-fg-faint focus:border-accent focus:outline-none disabled:opacity-60"
        />
        <Button type="submit" variant="primary" size="md" disabled={!canAsk || busy || !question.trim()}>
          {busy ? <Loader2 size={14} className="animate-spin" /> : <CornerDownLeft size={14} />}
          Ask
        </Button>
      </form>
      <p className="pt-1.5 text-[11px] text-fg-faint">
        Each question is answered on its own — <Kbd>{modKey()} J</Kbd> opens this from anywhere.
      </p>
    </div>
  );
}
