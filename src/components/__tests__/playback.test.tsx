import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PlayerBar } from "../PlayerBar";
import { TooltipProvider } from "../ui";
import type { AudioPlayer } from "../../hooks/useAudio";

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
