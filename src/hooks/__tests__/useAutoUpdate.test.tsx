import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useAutoUpdate } from "../useAutoUpdate";
import { checkForUpdate, installUpdate } from "../../lib/updater";

vi.mock("../../lib/updater", () => ({ checkForUpdate: vi.fn(), installUpdate: vi.fn() }));

function Harness({ safe }: { safe: boolean }) {
  const updater = useAutoUpdate(safe);
  return (
    <>
      {updater.update && (
        <button type="button" disabled={!safe} onClick={() => void updater.install()}>
          {safe ? "Update now" : "Stop recording to update"}
        </button>
      )}
      <output>{updater.update?.version ?? "none"}</output>
    </>
  );
}

describe("useAutoUpdate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("notifies about an available signed update and installs only after the user clicks", async () => {
    vi.useFakeTimers();
    const update = { version: "0.1.2", downloadAndInstall: vi.fn() };
    vi.mocked(checkForUpdate).mockResolvedValue({ kind: "available", update });
    vi.mocked(installUpdate).mockResolvedValue(undefined);
    render(<Harness safe />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });

    expect(screen.getByText("0.1.2")).toBeInTheDocument();
    expect(installUpdate).not.toHaveBeenCalled();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Update now" }));
      await Promise.resolve();
    });
    expect(installUpdate).toHaveBeenCalledWith(update, expect.any(Function));
  });

  it("checks while recording but blocks the relaunch until capture is idle", async () => {
    vi.useFakeTimers();
    const update = { version: "0.1.2", downloadAndInstall: vi.fn() };
    vi.mocked(checkForUpdate).mockResolvedValue({ kind: "available", update });
    render(<Harness safe={false} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });

    expect(checkForUpdate).toHaveBeenCalled();
    const button = screen.getByRole("button", { name: "Stop recording to update" });
    expect(button).toBeDisabled();
    expect(installUpdate).not.toHaveBeenCalled();
  });
});
