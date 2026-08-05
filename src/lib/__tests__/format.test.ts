import { describe, expect, it } from "vitest";
import { formatBytes } from "../format";

describe("formatBytes", () => {
  it("uses GB past a billion, so 1.6 GB does not read as 1625 MB", () => {
    expect(formatBytes(1624555275)).toBe("1.6 GB");
  });

  it("rounds MB to something a person would say", () => {
    expect(formatBytes(190085487)).toBe("190 MB");
  });

  it("never reports a real file as 0 KB", () => {
    expect(formatBytes(315894)).toBe("316 KB");
    expect(formatBytes(12)).toBe("1 KB");
  });
});
