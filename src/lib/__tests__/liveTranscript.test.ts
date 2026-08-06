import { describe, expect, it } from "vitest";
import type { LiveTranscriptEvent } from "../ipc";
import { isNearBottom, mergeLiveTranscript } from "../liveTranscript";

const partial = (speaker: "me" | "them", text: string): LiveTranscriptEvent => ({
  speaker,
  text,
  isPartial: true,
  isFinal: false,
});

describe("mergeLiveTranscript", () => {
  it("mutates the last partial for a speaker in place", () => {
    const first = partial("me", "hello");
    const next = mergeLiveTranscript([first], partial("me", "hello there"));
    expect(next).toEqual([partial("me", "hello there")]);
    expect(next[0]).not.toBe(first);
  });

  it("freezes a final line and starts the next utterance below it", () => {
    const final: LiveTranscriptEvent = {
      speaker: "me",
      text: "finished",
      isPartial: false,
      isFinal: true,
    };
    const frozen = mergeLiveTranscript([partial("me", "draft")], final);
    expect(mergeLiveTranscript(frozen, partial("me", "next"))).toEqual([
      final,
      partial("me", "next"),
    ]);
  });

  it("keeps independent partial slots for me and them", () => {
    const messages = mergeLiveTranscript([], partial("me", "mine"));
    const withThem = mergeLiveTranscript(messages, partial("them", "theirs"));
    expect(mergeLiveTranscript(withThem, partial("me", "mine updated"))).toEqual([
      partial("me", "mine updated"),
      partial("them", "theirs"),
    ]);
  });
});

describe("isNearBottom", () => {
  it("recognizes the bottom threshold", () => {
    expect(isNearBottom({ scrollHeight: 500, scrollTop: 402, clientHeight: 50 } as HTMLElement)).toBe(true);
    expect(isNearBottom({ scrollHeight: 500, scrollTop: 350, clientHeight: 50 } as HTMLElement)).toBe(false);
  });
});
