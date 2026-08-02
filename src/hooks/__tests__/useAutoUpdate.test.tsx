import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { useAutoUpdate } from "../useAutoUpdate";
import { checkForUpdate, installUpdate } from "../../lib/updater";

vi.mock("../../lib/updater", () => ({ checkForUpdate: vi.fn(), installUpdate: vi.fn() }));

function Harness({ safe }: { safe: boolean }) {
  useAutoUpdate(safe);
  return null;
}

describe("useAutoUpdate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("installs an available signed update automatically when recording is idle", async () => {
    vi.useFakeTimers();
    const update = { version: "0.1.2", downloadAndInstall: vi.fn() };
    vi.mocked(checkForUpdate).mockResolvedValue({ kind: "available", update });
    vi.mocked(installUpdate).mockResolvedValue(undefined);
    render(<Harness safe />);

    await vi.advanceTimersByTimeAsync(30_000);

    expect(installUpdate).toHaveBeenCalledWith(update, expect.any(Function));
  });

  it("does not check while a recording is active", async () => {
    vi.useFakeTimers();
    render(<Harness safe={false} />);

    await vi.advanceTimersByTimeAsync(30_000);

    expect(checkForUpdate).not.toHaveBeenCalled();
  });
});
