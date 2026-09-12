import { useEffect } from "react";
import type { ClipboardEntry } from "../types";

interface UseListSelectionResetOptions {
  filteredHistory: ClipboardEntry[];
  setSelectedIndex: (val: number) => void;
  // Index to land on when the list changes; raised above hidden pinned items
  // while the pinned section is collapsed. Use -1 if no visible entries remain.
  resetIndex?: number;
}

export const useListSelectionReset = ({
  filteredHistory,
  setSelectedIndex,
  resetIndex = 0
}: UseListSelectionResetOptions) => {
  useEffect(() => {
    setSelectedIndex(resetIndex < filteredHistory.length ? resetIndex : -1);
  }, [filteredHistory, setSelectedIndex, resetIndex]);
};
