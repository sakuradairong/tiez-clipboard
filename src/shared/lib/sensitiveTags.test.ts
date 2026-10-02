import { describe, expect, it } from "vitest";
import { hasSensitiveTag } from "./sensitiveTags";

describe("sensitive tags", () => {
  it.each(["sensitive", "Sensitive", "SENSITIVE", "password", "Password", "PASSWORD", "密码"])(
    "recognizes the backend-sensitive tag %s",
    (tag) => expect(hasSensitiveTag(["work", tag])).toBe(true)
  );

  it("does not treat absent or merely similar tags as sensitive", () => {
    expect(hasSensitiveTag()).toBe(false);
    expect(hasSensitiveTag([])).toBe(false);
    expect(hasSensitiveTag(["passwords", "nonsensitive", " sensitive "])).toBe(false);
  });
});
