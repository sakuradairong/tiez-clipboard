import { describe, expect, it } from "vitest";
import {
  selectNewestMainUiBootstrap,
  shouldCarryStableMainUiPhases,
  type MainUiBootstrap
} from "./useMainUiReadiness";

const bootstrap = (requestId: number | null, generation = 1): MainUiBootstrap => ({
  enabled: requestId != null,
  mode: requestId == null ? "default" : "destroyed",
  generation,
  request_id: requestId,
  intent: requestId == null ? null : "test"
});

describe("selectNewestMainUiBootstrap", () => {
  it("does not let a late bootstrap invoke replace a newer wake event", () => {
    const current = bootstrap(9, 4);
    expect(selectNewestMainUiBootstrap(current, bootstrap(null, 1))).toBe(current);
    expect(selectNewestMainUiBootstrap(current, bootstrap(8, 3))).toBe(current);
  });

  it("accepts the same or a newer correlated request", () => {
    const current = bootstrap(9, 4);
    const same = bootstrap(9, 4);
    const newer = bootstrap(10, 5);
    expect(selectNewestMainUiBootstrap(current, same)).toBe(same);
    expect(selectNewestMainUiBootstrap(current, newer)).toBe(newer);
  });
});

describe("shouldCarryStableMainUiPhases", () => {
  it("carries mounted and hydrated observations between hidden-mode wakes", () => {
    expect(shouldCarryStableMainUiPhases(bootstrap(11, 3), bootstrap(12, 3))).toBe(true);
  });

  it("requires a recreated generation to report mounted and hydrated again", () => {
    expect(shouldCarryStableMainUiPhases(bootstrap(11, 3), bootstrap(12, 4))).toBe(false);
  });
});
