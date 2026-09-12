import { useEffect } from "react";
import type { ClipboardEntry } from "../types";

interface UseListSelectionResetOptions {
  filteredHistory: ClipboardEntry[];
  setSelectedIndex: (val: number) => void;
  // Index to land on when the list changes; raised above hidden pinned items
  // while the pinned section is collapsed. Clamped to the list length.
  resetIndex?: number;
}

export const useListSelectionReset = ({
  filteredHistory,
  setSelectedIndex,
  resetIndex = 0
}: UseListSelectionResetOptions) => {
  useEffect(() => {
    setSelectedIndex(Math.min(resetIndex, Math.max(filteredHistory.length - 1, 0)));
  }, [filteredHistory, setSelectedIndex, resetIndex]);
};
