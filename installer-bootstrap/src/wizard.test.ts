import { describe, expect, it } from "vitest";
import { en, zh } from "./i18n";
import { nextStep, previousStep } from "./wizard";

describe("wizard steps", () => {
  it("walks welcome, location, progress, then finish", () => {
    expect(nextStep("welcome")).toBe("path");
    expect(nextStep("path")).toBe("progress");
    expect(nextStep("progress")).toBe("finish");
    expect(nextStep("finish")).toBe("finish");
  });

  it("returns to welcome only from the location step", () => {
    expect(previousStep("path")).toBe("welcome");
    expect(previousStep("progress")).toBe("progress");
    expect(previousStep("welcome")).toBe("welcome");
  });
});

describe("copy", () => {
  it("uses the same keys in Chinese and English", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort());
  });
});
