import { cleanup, render } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it } from "vitest";
import { EchoMark } from "../EchoMark";

// vitest globals are off in this project (see vite.config.ts), so
// testing-library's auto-cleanup never engages on its own — every other test
// file in this repo calls this explicitly, and skipping it here would leak
// each render into the next test's DOM.
afterEach(() => {
  cleanup();
});

describe("EchoMark", () => {
  // DOM ids must be unique. Before useId(), both instances shared the literal
  // id "echo-bg", so the second gradient definition silently won the id and
  // the first mark's path referenced a gradient it did not own. Rendering two
  // instances together is the only way this bug shows up — a single instance
  // always looks correct.
  it("gives two instances distinct gradient ids that each match their own path's fill", () => {
    const { container } = render(
      <>
        <EchoMark />
        <EchoMark />
      </>,
    );

    const svgs = container.querySelectorAll("svg");
    expect(svgs).toHaveLength(2);

    const marks = Array.from(svgs).map((svg) => ({
      gradientId: svg.querySelector("linearGradient")?.getAttribute("id"),
      pathFill: svg.querySelector("path")?.getAttribute("fill"),
    }));
    const [first, second] = marks;

    expect(first.gradientId).toBeTruthy();
    expect(second.gradientId).toBeTruthy();
    expect(first.gradientId).not.toBe(second.gradientId);
    expect(first.pathFill).toBe(`url(#${first.gradientId})`);
    expect(second.pathFill).toBe(`url(#${second.gradientId})`);
  });
});
