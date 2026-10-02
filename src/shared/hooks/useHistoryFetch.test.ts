import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipboardEntry } from "../types";
import { useHistoryFetch } from "./useHistoryFetch";

const hooks = vi.hoisted(() => ({ refs: [] as Array<{ current: unknown }>, cursor: 0 }));
vi.mock("react", () => ({
  useRef: (current: unknown) => hooks.refs[hooks.cursor++] ??
    (hooks.refs[hooks.cursor - 1] = { current }),
  useEffect: (effect: () => void) => effect(),
  useCallback: (callback: unknown) => callback
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
import { invoke } from "@tauri-apps/api/core";

const entries = (count: number, session = false) => Array.from({ length: count }, (_, i) => ({
  id: session ? -i - 1 : i + 1, content: `item ${i}`, content_type: "text"
})) as ClipboardEntry[];

function setup(session: ClipboardEntry[], persisted: ClipboardEntry[]) {
  let history: ClipboardEntry[] = [];
  let offset = 0;
  let hasMore = true;
  const setHistory = (value: ClipboardEntry[] | ((prev: ClipboardEntry[]) => ClipboardEntry[])) => {
    history = typeof value === "function" ? value(history) : value;
  };
  const setOffset = (value: number | ((prev: number) => number)) => {
    offset = typeof value === "function" ? value(offset) : value;
  };
  vi.mocked(invoke).mockImplementation(async (_command, args) => {
    const page = args as { limit: number; offset: number; includeAllSession: boolean };
    expect(page.includeAllSession).toBe(true);
    return [
      ...(page.offset === 0 ? session : []),
      ...persisted.slice(page.offset, page.offset + page.limit)
    ] as never;
  });
  const render = () => {
    hooks.cursor = 0;
    return useHistoryFetch({
      debouncedSearch: "", typeFilter: null, pageSize: 80,
      persistentLimit: 1000, persistentLimitEnabled: true,
      currentOffset: offset, historyLength: history.length,
      setHistory, setCurrentOffset: setOffset, setHasMore: (value) => {
        hasMore = typeof value === "function" ? value(hasMore) : value;
      },
      isLoadingMore: false, hasMore, setIsLoadingMore: () => {}
    });
  };
  return { render, history: () => history, offset: () => offset, hasMore: () => hasMore };
}

beforeEach(() => {
  hooks.refs = [];
  hooks.cursor = 0;
  vi.mocked(invoke).mockReset();
});

describe("session and persisted history pagination", () => {
  it("keeps all 200 session entries after reset without a phantom next page", async () => {
    const state = setup(entries(200, true), []);
    await state.render().fetchHistory(true);
    expect(state.history()).toHaveLength(200);
    expect(state.offset()).toBe(0);
    expect(state.hasMore()).toBe(false);
    await state.render().loadMoreHistory();
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("loads DB pages alongside every session entry, including after another reset", async () => {
    const state = setup(entries(200, true), entries(170));
    await state.render().fetchHistory(true);
    expect(state.history()).toHaveLength(280);
    expect(state.offset()).toBe(80);
    expect(state.hasMore()).toBe(true);
    await state.render().loadMoreHistory();
    expect(state.history()).toHaveLength(360);
    expect(state.offset()).toBe(160);
    await state.render().loadMoreHistory();
    expect(state.history()).toHaveLength(370);
    expect(state.hasMore()).toBe(false);
    await state.render().fetchHistory(true);
    await state.render().loadMoreHistory();
    expect(state.history()).toHaveLength(360);
    expect(vi.mocked(invoke).mock.calls.map((call) => (call[1] as { offset: number }).offset))
      .toEqual([0, 80, 160, 0, 80]);
  });

  it("counts session entries converted to positive IDs only once in the DB page", async () => {
    const session = entries(90, true);
    session[0] = { ...session[0], id: 1 };
    session[1] = { ...session[1], id: 2 };
    // The backend deduplicates converted IDs before returning the first page.
    const state = setup(session.filter((entry) => entry.id < 0), entries(100));
    await state.render().fetchHistory(true);
    await state.render().loadMoreHistory();
    expect(state.history()).toHaveLength(188);
    expect(new Set(state.history().map((entry) => entry.id)).size).toBe(188);
    expect(state.offset()).toBe(100);
  });
});
