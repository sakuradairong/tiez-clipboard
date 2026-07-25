import { describe, expect, it } from "vitest";
import { githubHttpsUrlOrFallback } from "./fork";

const fallback = "https://github.com/example/project";

describe("GitHub opener URL validation", () => {
  it("accepts only GitHub HTTPS URLs", () => {
    expect(
      githubHttpsUrlOrFallback(" https://github.com/example/project/issues ", fallback)
    ).toBe("https://github.com/example/project/issues");
  });

  it("normalizes only pathname trailing slashes", () => {
    expect(
      githubHttpsUrlOrFallback(
        "https://github.com/example/project/?tab=readme#intro",
        fallback
      )
    ).toBe("https://github.com/example/project?tab=readme#intro");
    expect(
      githubHttpsUrlOrFallback(
        "https://github.com/example/project/issues/?q=open#results",
        fallback
      )
    ).toBe("https://github.com/example/project/issues?q=open#results");
  });

  it.each([
    "http://github.com/example/project",
    "https://gitlab.com/example/project",
    "https://github.com.evil.example/example/project",
    "https://github.com:8443/example/project",
    "not a url",
  ])("falls back for opener-incompatible URL %s", (value) => {
    expect(githubHttpsUrlOrFallback(value, fallback)).toBe(fallback);
  });
});
