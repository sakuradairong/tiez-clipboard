import { afterEach, describe, expect, it, vi } from "vitest";
import { detectsWindowsPlatform, isWindowsPlatform } from "./platform";

describe("platform detection", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("detects Windows from either the user agent or navigator platform", () => {
    expect(detectsWindowsPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "Linux x86_64"))
      .toBe(true);
    expect(detectsWindowsPlatform("Mozilla/5.0", "Win32")).toBe(true);
  });

  it("does not expose the Win+V setting on macOS or Linux", () => {
    expect(detectsWindowsPlatform("Mozilla/5.0 (Macintosh)", "MacIntel")).toBe(false);
    expect(detectsWindowsPlatform("Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64"))
      .toBe(false);
  });

  it("is safe when rendered without a browser navigator", () => {
    vi.stubGlobal("navigator", undefined);
    expect(isWindowsPlatform()).toBe(false);
  });
});
