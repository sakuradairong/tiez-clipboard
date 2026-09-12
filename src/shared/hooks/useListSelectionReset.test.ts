import { describe, expect, it, vi } from "vitest";
import type { ClipboardEntry } from "../types";
import { useListSelectionReset } from "./useListSelectionReset";

vi.mock("react", () => ({ useEffect: (effect: () => void) => effect() }));

describe("useListSelectionReset", () => {
  it.each([
    { length: 0, resetIndex: 0, expected: -1 },
    { length: 2, resetIndex: 2, expected: -1 },
    { length: 2, resetIndex: 3, expected: -1 },
    { length: 3, resetIndex: 2, expected: 2 },
    { length: 2, resetIndex: 0, expected: 0 }
  ])("resets length=$length, base=$resetIndex to $expected", ({ length, resetIndex, expected }) => {
    const setSelectedIndex = vi.fn();
    useListSelectionReset({
      filteredHistory: Array.from({ length }, (_, id) => ({ id }) as ClipboardEntry),
      resetIndex,
      setSelectedIndex
    });
    expect(setSelectedIndex).toHaveBeenCalledWith(expected);
  });
});
