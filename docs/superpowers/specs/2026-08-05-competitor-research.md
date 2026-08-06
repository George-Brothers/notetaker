# Competitor research — meeting notetakers (2026-08-05)

Commissioned by Mr. Brothers ("look up competitors and best meeting note taker
apps — see what they have and what functionality I can learn from them") while
designing the tray menu expansion and the floating overlay. Web research,
2026-08-05. Items 5 and elements of 10 shipped the same night (Krisp-style
pill content; content-protected overlay).

## Per-app findings

**Granola** (closest analog — local capture, no bot; Mac + Windows)
- Menu bar: upcoming meetings from calendar plus a "quick note" button; the
  app is menu-bar-first, main window optional.
- Overlay: small draggable "live meeting indicator" pill on the right screen
  edge — recording state, click returns to the notepad.
- Detection: **mic-in-use detection**, not just app detection — notification
  titled per platform ("Huddle detected" for Slack, "Call detected" for
  FaceTime/WhatsApp). Calendar-linked meetings prompt at start time;
  back-to-back calls are detected individually, notes pre-created per meeting.
- Meeting end: multi-signal auto-stop — call app no longer in use, transcript
  activity, calendar end time, 15 min silence, system sleep. Then "Enhance
  notes": AI merges typed bullets with the transcript; your text stays black,
  AI additions render gray.

**Otter.ai** — desktop app records bot-free; auto-start/auto-end, persistent
pause/end indicator. Weak Mac menu-bar integration (a gap we already beat).
Slide-capture into transcripts is a cloud-side differentiator idea.

**Fathom** — live panel docked beside Zoom: recording controls plus a
**Highlight button** that bookmarks the moment with an optional quick note;
highlights become clips. Panel hidden from screen shares. Their bot-free mode
lost in-call highlights — a live-capture app that keeps them leapfrogs them.
Per-meeting capture modes (transcript-only / audio / audio+video). Meeting
end is their flagship: summary + action items ready the moment you hang up.

**Fireflies** — collapsible side panel; manual notes typed during the meeting
merge with AI notes after. Granular auto-join rules (all meetings vs
invite-only) — the "rules per meeting type" pattern for auto-record.

**tl;dv** — minimal in-meeting UI, "invisible until done": recordings process
automatically the moment the call ends. Documents the anti-pattern: raw
system-audio capture records notification dings — our dual-track capture
doesn't pollute the far-side track, worth advertising.

**Krisp** — the best floating-widget reference: auto-appears when a call
starts (toggleable), meeting duration, note-taker status, pause/resume, and
live stats — noise removed, **talk-time vs listen-time ratio**. Mic-usage
detection, app-agnostic. Auto-opens the notes page right after meeting end.

**MacWhisper** (local-first, Mac) — menu-bar-only mode; meeting detection
across a long app list with "Meeting Detected"/"Meeting Ended" notifications
carrying Record/Stop buttons. Post-transcribe automations: push transcript to
Notion/Obsidian/webhook — the local-first substitute for cloud integrations.

**Superwhisper** — local meeting mode exists but thin; confirms the space for
a dedicated local meeting app.

**Notion AI Meeting Notes** — mic-in-use "a call is starting — transcribe?"
notification; meeting-type formats (Auto / Sales / Stand-up / Team) change the
summary structure — per-meeting summary templates are very stealable.

**Zoom AI Companion** — live in-meeting AI queries over the running
transcript: "Catch me up," "Was my name mentioned?", "Action items so far?"
The standout feature nobody local has; entirely doable with a local LLM.

## Steal this — ranked

1. **Live "catch me up"** over the running transcript (Zoom) — overlay button
   answering "what did I miss?" from the local LLM. The single most
   impressive demo a local app can do.
2. **Highlight button** on the overlay (Fathom) — one click bookmarks the
   timestamp with an optional note; global hotkey too. Fathom's bot-free mode
   dropped this; we'd leapfrog.
3. **Bullets-then-enhance notes** (Granola/Fireflies) — notepad reachable from
   the overlay; at meeting end the LLM merges bullets with transcript, AI
   additions in a distinct color. (Our notes/summary split already half-does
   this — the overlay entry point is the missing piece.)
4. **Meeting-ended notification with instant payoff** (Fathom/MacWhisper) — on
   auto-stop, a notification that opens straight to summary + actions.
5. **Krisp-style widget content** — duration, state, pause/resume, and a live
   talk-vs-listen ratio from our two tracks (nobody local shows this).
   *Shipped 2026-08-05 (minus the ratio).*
6. **Upcoming meetings in the tray** (Granola) — next 2–3 calendar events,
   click to pre-create its note. Needs local calendar read.
7. **Multi-signal auto-stop** (Granola) — app quit + calendar end + N min
   silence + sleep. Recording 3 hours after the call is the trust-killer.
8. **Mic-in-use detection with per-platform labels** (Granola/Notion) —
   catches FaceTime/huddles/browser calls the app-watcher misses.
9. **Per-meeting capture rules** (Fathom/Fireflies) — defaults plus overrides,
   "auto-record external, ask for internal."
10. **Hide-from-screen-share overlay** (Fathom) *(shipped 2026-08-05)* +
    **post-meeting export automations** (MacWhisper) — auto-write finished
    notes to an Obsidian/markdown folder or user webhook.

Cross-cutting: Granola's edge is the tray + detection + enhance loop (items
3/6/7/8 neutralize it); Fathom owns meeting-end (4); Krisp owns the widget
(5); Zoom owns live Q&A (1) — the one feature that would put a local app
*ahead* of the cloud players.
