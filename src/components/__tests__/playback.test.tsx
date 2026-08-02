import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PlayerBar } from "../PlayerBar";
import { TranscriptPanel } from "../TranscriptPanel";
import { NoteView } from "../NoteView";
import { TooltipProvider } from "../ui";
import { liveRecordingId } from "../../App";
import type { AudioPlayer } from "../../hooks/useAudio";
import type { RecordingDetail } from "../../lib/ipc";

afterEach(cleanup);

function player(over: Partial<AudioPlayer> = {}): AudioPlayer {
  return {
    ref: { current: null },
    src: "blob:audio",
    error: null,
    playing: false,
    setPlaying: vi.fn(),
    currentTime: 30,
    setCurrentTime: vi.fn(),
    seek: vi.fn(),
    seekAndPlay: vi.fn(),
    skip: vi.fn(),
    toggle: vi.fn(),
    rate: 1,
    setRate: vi.fn(),
    ...over,
  };
}

function draw(props: Partial<React.ComponentProps<typeof PlayerBar>> = {}) {
  const audio = props.audio ?? player();
  const onCollapse = props.onCollapse ?? vi.fn();
  const onTrackChange = props.onTrackChange ?? vi.fn();
  render(
    <TooltipProvider>
      <PlayerBar
        audio={audio}
        durationS={600}
        tracks={["mic"]}
        track="mic"
        onTrackChange={onTrackChange}
        onCollapse={onCollapse}
        live={false}
        {...props}
      />
    </TooltipProvider>,
  );
  return { audio, onCollapse, onTrackChange };
}

describe("PlayerBar", () => {
  it("skips back ten seconds", () => {
    const { audio } = draw();
    fireEvent.click(screen.getByRole("button", { name: /back 10 seconds/i }));
    expect(audio.skip).toHaveBeenCalledWith(-10);
  });

  it("skips forward ten seconds", () => {
    const { audio } = draw();
    fireEvent.click(screen.getByRole("button", { name: /forward 10 seconds/i }));
    expect(audio.skip).toHaveBeenCalledWith(10);
  });

  it("restarts without changing whether it is playing", () => {
    const { audio } = draw();
    fireEvent.click(screen.getByRole("button", { name: /back to the start/i }));
    expect(audio.seek).toHaveBeenCalledWith(0);
    expect(audio.toggle).not.toHaveBeenCalled();
  });

  it("plays and pauses from the same button", () => {
    const { audio } = draw();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    expect(audio.toggle).toHaveBeenCalled();
  });

  it("shows Pause while it is playing", () => {
    draw({ audio: player({ playing: true }) });
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
  });

  it("shows elapsed against total", () => {
    draw();
    expect(screen.getByText("0:30 / 10:00")).toBeInTheDocument();
  });

  it("scrubs to wherever the slider is dragged", () => {
    const { audio } = draw();
    fireEvent.change(screen.getByRole("slider", { name: /position/i }), {
      target: { value: "120" },
    });
    expect(audio.seek).toHaveBeenCalledWith(120);
  });

  it("changes speed", () => {
    const { audio } = draw();
    fireEvent.click(screen.getByRole("button", { name: /playback speed/i }));
    fireEvent.click(screen.getByRole("button", { name: "1.5×" }));
    expect(audio.setRate).toHaveBeenCalledWith(1.5);
  });

  it("names the only track without offering a chooser", () => {
    draw();
    expect(screen.getByText("Your microphone")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /which track/i })).not.toBeInTheDocument();
  });

  it("offers a chooser when more than one track has audio", () => {
    const { onTrackChange } = draw({ tracks: ["mic", "system"], track: "mic" });
    fireEvent.click(screen.getByRole("button", { name: /which track/i }));
    fireEvent.click(screen.getByRole("button", { name: "Everyone else" }));
    expect(onTrackChange).toHaveBeenCalledWith("system");
  });

  it("minimises back into the toolbar", () => {
    const { onCollapse } = draw();
    fireEvent.click(screen.getByRole("button", { name: /minimise/i }));
    expect(onCollapse).toHaveBeenCalled();
  });

  it("says so plainly when no audio was saved", () => {
    draw({ tracks: [], track: null });
    expect(screen.getByText("No audio was saved for this recording.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Play" })).not.toBeInTheDocument();
  });

  it("says so plainly while the recording is still going", () => {
    draw({ live: true });
    expect(
      screen.getByText("This recording is still going. It'll be listenable as soon as you stop."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Play" })).not.toBeInTheDocument();
  });

  it("still offers a way out when there is nothing to play", () => {
    const { onCollapse } = draw({ tracks: [], track: null });
    fireEvent.click(screen.getByRole("button", { name: /minimise/i }));
    expect(onCollapse).toHaveBeenCalled();
  });

  it("takes space and the arrow keys, but not from a text field", () => {
    const { audio } = draw();
    const bar = screen.getByRole("group", { name: /playback/i });

    fireEvent.keyDown(bar, { key: " " });
    expect(audio.toggle).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(bar, { key: "ArrowLeft" });
    expect(audio.skip).toHaveBeenCalledWith(-10);
    fireEvent.keyDown(bar, { key: "ArrowRight" });
    expect(audio.skip).toHaveBeenCalledWith(10);

    // The scrubber is an input: arrow keys are its own, and space must not
    // be stolen from anything a person can type into.
    fireEvent.keyDown(screen.getByRole("slider", { name: /position/i }), { key: " " });
    expect(audio.toggle).toHaveBeenCalledTimes(1);
  });
});

function detailWith(over: Partial<RecordingDetail> = {}): RecordingDetail {
  return {
    id: "rec-1",
    title: "Team sync",
    task: null,
    created: "2026-07-30T14:00:00Z",
    durationS: 600,
    mode: "meeting",
    status: "ready",
    hasNotes: false,
    transcriptMd: "",
    summaryMd: "",
    notesMd: "",
    error: null,
    captureNote: null,
    suggestedTask: null,
    suggestedTitle: null,
    template: null,
    actions: [],
    speakers: {},
    segments: [
      { line: 0, startS: 0, endS: 5, speaker: "Speaker 1", text: "Morning." },
      { line: 1, startS: 5, endS: 9, speaker: "Speaker 2", text: "How are you?" },
    ],
    audioTracks: ["mic"],
    ...over,
  } as RecordingDetail;
}

describe("TranscriptPanel", () => {
  it("seeks the shared player when a line is clicked, and starts it", () => {
    const audio = player();
    render(
      <TooltipProvider>
        <TranscriptPanel detail={detailWith()} audio={audio} onRenameSpeaker={vi.fn()} />
      </TooltipProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /Morning\./i }));
    expect(audio.seekAndPlay).toHaveBeenCalledWith(0);
  });

  it("owns no audio element of its own", () => {
    const { container } = render(
      <TooltipProvider>
        <TranscriptPanel detail={detailWith()} audio={player()} onRenameSpeaker={vi.fn()} />
      </TooltipProvider>,
    );
    expect(container.querySelector("audio")).toBeNull();
  });
});

function note(over: Partial<React.ComponentProps<typeof NoteView>> = {}) {
  const props: React.ComponentProps<typeof NoteView> = {
    detail: detailWith(),
    loading: false,
    tasks: [],
    templates: [],
    askOpen: false,
    liveRecordingId: null,
    onToggleAsk: vi.fn(),
    onRenameSpeaker: vi.fn(),
    onSaveSummary: vi.fn(),
    onRenameRecording: vi.fn(),
    onAssignTask: vi.fn(),
    onSaveNotes: vi.fn().mockResolvedValue(undefined),
    onSetTemplate: vi.fn(),
    onToggleAction: vi.fn(),
    onProcessNow: vi.fn(),
    onBack: vi.fn(),
    ...over,
  };
  render(
    <TooltipProvider>
      <NoteView {...props} />
    </TooltipProvider>,
  );
}

describe("Listen", () => {
  it("is closed until you press it", () => {
    note();
    expect(screen.queryByRole("group", { name: /playback/i })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    expect(screen.getByRole("group", { name: /playback/i })).toBeInTheDocument();
  });

  it("opens on a recording that has never been processed", () => {
    // The regression this whole feature exists for: no segments, no
    // transcript, and audio sitting on disk the entire time.
    note({
      detail: detailWith({
        status: "recorded",
        segments: [],
        transcriptMd: "",
        audioTracks: ["mic"],
      }),
    });
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    expect(screen.getByRole("button", { name: "Play" })).toBeInTheDocument();
  });

  it("minimises back into the toolbar", () => {
    note();
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    fireEvent.click(screen.getByRole("button", { name: /minimise/i }));
    expect(screen.queryByRole("group", { name: /playback/i })).not.toBeInTheDocument();
  });

  it("opens itself when you go to the transcript", () => {
    note();
    fireEvent.mouseDown(screen.getByRole("tab", { name: /transcript/i }));
    expect(screen.getByRole("group", { name: /playback/i })).toBeInTheDocument();
  });

  it("keeps exactly one audio element however it was opened", async () => {
    note();
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    fireEvent.mouseDown(screen.getByRole("tab", { name: /transcript/i }));
    await waitFor(() => expect(document.querySelectorAll("audio")).toHaveLength(1));
  });

  it("does not stop playing when it is minimised", async () => {
    // "Minimise", not "close". The element has to outlive the bar.
    note();
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    await waitFor(() => expect(document.querySelectorAll("audio")).toHaveLength(1));
    fireEvent.click(screen.getByRole("button", { name: /minimise/i }));
    expect(screen.queryByRole("group", { name: /playback/i })).not.toBeInTheDocument();
    expect(document.querySelectorAll("audio")).toHaveLength(1);
  });

  it("refuses plainly while that recording is still capturing", () => {
    note({ detail: detailWith({ id: "rec-1" }), liveRecordingId: "rec-1" });
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    expect(
      screen.getByText("This recording is still going. It'll be listenable as soon as you stop."),
    ).toBeInTheDocument();
  });

  it("does not refuse a different recording during that same capture", () => {
    note({ detail: detailWith({ id: "rec-1" }), liveRecordingId: "rec-9" });
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    expect(screen.getByRole("button", { name: "Play" })).toBeInTheDocument();
  });

  it("says so when the recording saved no audio at all", () => {
    note({ detail: detailWith({ audioTracks: [] }) });
    fireEvent.click(screen.getByRole("button", { name: /listen/i }));
    expect(screen.getByText("No audio was saved for this recording.")).toBeInTheDocument();
  });
});

describe("liveRecordingId", () => {
  it("is null while nothing is being captured", () => {
    expect(liveRecordingId({ state: "idle", recordingId: null })).toBeNull();
  });

  it("names the recording while it is being captured", () => {
    expect(liveRecordingId({ state: "recording", recordingId: "rec-1" })).toBe("rec-1");
  });

  it("still names it while paused", () => {
    expect(liveRecordingId({ state: "paused", recordingId: "rec-1" })).toBe("rec-1");
  });

  it("still names it while finishing", () => {
    expect(liveRecordingId({ state: "finishing", recordingId: "rec-1" })).toBe("rec-1");
  });
});
