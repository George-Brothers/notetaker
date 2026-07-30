/**
 * Tests for the transport switch.
 *
 * The interesting cases are all failure cases: a served UI runs on a phone that
 * walks out of wifi range, against a laptop that goes to sleep, and every one of
 * those has to produce a sentence a person can act on rather than "failed to
 * fetch".
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { accessToken, invoke, isDesktop } from "./transport";

const originalFetch = globalThis.fetch;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

beforeEach(() => {
  window.sessionStorage.clear();
  history.replaceState(null, "", "/");
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe("isDesktop", () => {
  it("is false in a plain browser", () => {
    expect(isDesktop()).toBe(false);
  });

  it("is true when the Tauri shell is present", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(isDesktop()).toBe(true);
  });
});

describe("accessToken", () => {
  it("is null when there is none", () => {
    expect(accessToken()).toBeNull();
  });

  it("reads a token out of the query string", () => {
    history.replaceState(null, "", "/?token=abc123");
    expect(accessToken()).toBe("abc123");
  });

  /** The link is opened once; moving around the app must not lose the code. */
  it("remembers the token after it leaves the url", () => {
    history.replaceState(null, "", "/?token=abc123");
    expect(accessToken()).toBe("abc123");
    history.replaceState(null, "", "/settings");
    expect(accessToken()).toBe("abc123");
  });

  it("prefers a fresh token in the url over a remembered one", () => {
    window.sessionStorage.setItem("notetaker.token", "old");
    history.replaceState(null, "", "/?token=new");
    expect(accessToken()).toBe("new");
  });
});

describe("invoke over HTTP", () => {
  it("posts the command name and arguments as JSON", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(["Accounting 302"]));
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    const result = await invoke<string[]>("create_task", { name: "Accounting 302" });

    expect(result).toEqual(["Accounting 302"]);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/create_task");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body)).toEqual({ name: "Accounting 302" });
  });

  it("sends an empty object when a command takes no arguments", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse([]));
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    await invoke("list_tasks");

    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({});
  });

  it("attaches the access token when there is one", async () => {
    history.replaceState(null, "", "/?token=secret-code");
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse([]));
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    await invoke("list_tasks");

    expect(fetchMock.mock.calls[0][1].headers["X-Notetaker-Token"]).toBe(
      "secret-code",
    );
  });

  it("sends no token header when there is no token", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse([]));
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    await invoke("list_tasks");

    expect(fetchMock.mock.calls[0][1].headers).not.toHaveProperty(
      "X-Notetaker-Token",
    );
  });

  /** The core writes its errors for a non-engineer; they must survive intact. */
  it("surfaces the error message the core wrote", async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        jsonResponse(
          { error: "That recording is still being written, so it cannot be moved yet." },
          400,
        ),
      ) as unknown as typeof fetch;

    await expect(invoke("assign_task", { id: "x", task: "y" })).rejects.toThrow(
      "That recording is still being written, so it cannot be moved yet.",
    );
  });

  it("explains a missing access code rather than showing a status number", async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(jsonResponse({ error: "nope" }, 401)) as unknown as typeof fetch;

    await expect(invoke("list_tasks")).rejects.toThrow(/access code/i);
  });

  /**
   * The most likely failure for a phone reading the library: the laptop slept.
   * "Failed to fetch" is not something a person can act on.
   */
  it("explains a dead connection in terms of the computer, not the network stack", async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new TypeError("Failed to fetch")) as unknown as typeof fetch;

    await expect(invoke("list_tasks")).rejects.toThrow(/asleep or off the network/i);
  });

  it("reports an unreadable reply instead of throwing a parse error", async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(new Response("<html>not json</html>", { status: 200 })) as unknown as typeof fetch;

    await expect(invoke("list_tasks")).rejects.toThrow(/could not read/i);
  });

  /** `void` commands return an empty body; that must not be an error. */
  it("handles a command that returns nothing", async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(new Response("", { status: 200 })) as unknown as typeof fetch;

    await expect(invoke("process_now", { id: "x" })).resolves.toBeNull();
  });

  it("falls back to a generic message when an error carries no text", async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(jsonResponse({}, 500)) as unknown as typeof fetch;

    await expect(invoke("list_tasks")).rejects.toThrow(/something went wrong/i);
  });
});
