import { useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauriRuntime } from "../lib/tauriRuntime";
import type { Dispatch, SetStateAction } from "react";
import type { ClipboardEntry } from "../types";

interface UseHistoryFetchOptions {
  debouncedSearch: string;
  typeFilter: string | null;
  persistentLimitEnabled: boolean;
  persistentLimit: number;
  pageSize: number;
  currentOffset: number;
  historyLength: number;
  setHistory: Dispatch<SetStateAction<ClipboardEntry[]>>;
  setCurrentOffset: Dispatch<SetStateAction<number>>;
  setHasMore: Dispatch<SetStateAction<boolean>>;
  isLoadingMore: boolean;
  hasMore: boolean;
  setIsLoadingMore: Dispatch<SetStateAction<boolean>>;
}

export const useHistoryFetch = ({
  debouncedSearch,
  typeFilter,
  persistentLimitEnabled,
  persistentLimit,
  pageSize,
  currentOffset,
  historyLength,
  setHistory,
  setCurrentOffset,
  setHasMore,
  isLoadingMore,
  hasMore,
  setIsLoadingMore
}: UseHistoryFetchOptions) => {
  const loadingRef = useRef(false);
  const fetchSeqRef = useRef(0);
  const lastRequestedOffsetRef = useRef<number | null>(null);
  const currentOffsetRef = useRef(currentOffset);
  const historyLengthRef = useRef(historyLength);
  // The config-created window can load before Rust setup finishes managing
  // DbState (likely right after boot), so the first invoke may fail. Retry
  // with backoff instead of leaving the history list empty until the next
  // clipboard event.
  const fetchAttemptsRef = useRef(0);
  const fetchFnRef = useRef<((reset?: boolean) => Promise<void>) | null>(null);
  const readyPollRef = useRef<number | null>(null);

  useEffect(() => {
    currentOffsetRef.current = currentOffset;
  }, [currentOffset]);

  useEffect(() => {
    historyLengthRef.current = historyLength;
  }, [historyLength]);
  const fetchHistory = useCallback(
    async (reset = false) => {
      if (!isTauriRuntime()) {
        return;
      }

      const seq = ++fetchSeqRef.current;
      try {
        if (reset) {
          lastRequestedOffsetRef.current = null;
        }

        const baseOffset = reset
          ? 0
          : Math.min(currentOffsetRef.current, historyLengthRef.current);

        let data: ClipboardEntry[] = [];

        const hasSearch = debouncedSearch && debouncedSearch.trim().length > 0;

        if (hasSearch) {
          let term = debouncedSearch;
          let tagOnly = false;
          if (term.startsWith("tag:")) {
            term = term.slice(4);
            tagOnly = true;
          }

          try {
            data = await invoke<ClipboardEntry[]>("search_clipboard_history", {
              searchTerm: term,
              limit: 200,
              tagOnly
            });
          } catch (e) {
            console.error("Search failed, falling back", e);
            data = [];
          }

          if (seq !== fetchSeqRef.current) return;
          // Search results are not paginated; always replace list and stop infinite loading.
          setHistory(data);
          setCurrentOffset(data.length);
          setHasMore(false);
        } else {
          const requestedLimit = pageSize + 1; // Use standard page size for DB limit
          const rawData = await invoke<ClipboardEntry[]>("get_clipboard_history", {
            limit: requestedLimit,
            offset: baseOffset,
            contentType: typeFilter || undefined,
            includeAllSession: true
          });

          if (seq !== fetchSeqRef.current) return;

          // Session history is bounded and returned in full on the first page.
          // Only persisted rows participate in the DB offset and lookahead.
          const hasMoreNow = rawData.filter((item) => item.id > 0).length > pageSize;
          let dbItemsCount = 0;
          const data = rawData.filter((item) => {
            if (item.id <= 0) return true;
            return dbItemsCount++ < pageSize;
          });
          dbItemsCount = Math.min(dbItemsCount, pageSize);

          if (reset) {
            setHistory(data);
            setCurrentOffset(dbItemsCount);
            setHasMore(hasMoreNow);
          } else {
            let nextItems: ClipboardEntry[] = [];
            setHistory((prev) => {
              const existingIds = new Set(prev.map((item) => item.id));
              nextItems = data.filter((item) => !existingIds.has(item.id) || item.id === 0);

              if (nextItems.length === 0) return prev;
              return [...prev, ...nextItems];
            });

            setCurrentOffset(prev => prev + dbItemsCount);
            // If we didn't add any NEW items but the backend says there are more,
            // it means the items we got were already in our list (maybe shifted due to sorting).
            // We should keep hasMore true so the user can try to load further.
            setHasMore(hasMoreNow);
          }
        }
        fetchAttemptsRef.current = 0;
        if (readyPollRef.current !== null) {
          window.clearInterval(readyPollRef.current);
          readyPollRef.current = null;
        }
      } catch (err) {
        console.error("无法获取历史记录", err);
        if (seq !== fetchSeqRef.current) return;
        setHasMore(false);

        const attempt = ++fetchAttemptsRef.current;
        if (attempt <= 5) {
          const delay = Math.min(300 * 2 ** (attempt - 1), 5000);
          window.setTimeout(() => {
            // Only retry if no newer fetch has started since the schedule.
            if (seq === fetchSeqRef.current) {
              void fetchFnRef.current?.(reset);
            }
          }, delay);
        } else if (readyPollRef.current === null) {
          // Retries exhausted: fall back to polling `is_app_ready`. This also
          // recovers when `app-ready` fired before the listener attached.
          readyPollRef.current = window.setInterval(() => {
            invoke<boolean>("is_app_ready")
              .then((ready) => {
                if (!ready) return;
                if (readyPollRef.current !== null) {
                  window.clearInterval(readyPollRef.current);
                  readyPollRef.current = null;
                }
                void fetchFnRef.current?.(true);
              })
              .catch(() => {
                // Backend state not managed yet; keep polling.
              });
          }, 2000);
        }
      }
    },
    [
      debouncedSearch,
      typeFilter,
      pageSize,
      persistentLimit,
      persistentLimitEnabled,
      setCurrentOffset,
      setHasMore,
      setHistory
    ]
  );

  fetchFnRef.current = fetchHistory;

  useEffect(() => {
    if (!isTauriRuntime()) return;

    // Rust setup emits `app-ready` once all backend state is managed; refetch
    // so a slow boot still shows history even after fetch retries were spent.
    const unlistenPromise = listen("app-ready", () => {
      void fetchFnRef.current?.(true);
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
      if (readyPollRef.current !== null) {
        window.clearInterval(readyPollRef.current);
        readyPollRef.current = null;
      }
    };
  }, []);

  const loadMoreHistory = useCallback(async () => {
    if (loadingRef.current || isLoadingMore || !hasMore) return;
    if (debouncedSearch && debouncedSearch.trim().length > 0) return;

    const effectiveOffset = Math.min(currentOffsetRef.current, historyLengthRef.current);
    if (lastRequestedOffsetRef.current === effectiveOffset) return;
    lastRequestedOffsetRef.current = effectiveOffset;

    loadingRef.current = true;
    setIsLoadingMore(true);
    try {
      await fetchHistory(false);
    } finally {
      loadingRef.current = false;
      setIsLoadingMore(false);
    }
  }, [debouncedSearch, fetchHistory, hasMore, isLoadingMore, setIsLoadingMore]);

  return { fetchHistory, loadMoreHistory };
};
