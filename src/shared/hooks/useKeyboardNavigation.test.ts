import { afterEach, describe, expect, it, vi } from "vitest";
import type { ClipboardEntry } from "../types";
import { useKeyboardNavigation } from "./useKeyboardNavigation";

const native = vi.hoisted(() => ({
  handler: undefined as ((event: { payload: string }) => Promise<void>) | undefined
}));

// Exercise the hook's event handlers without a browser or native Tauri runtime.
// Each render installs handlers with the latest state; effects run synchronously.
vi.mock("react", () => ({
  useRef: (current: unknown) => ({ current }),
  useEffect: (effect: () => void) => effect()
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((_name: string, handler: typeof native.handler) => {
    native.handler = handler;
    return Promise.resolve(() => {});
  })
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ isVisible: async () => true })
}));
vi.mock("./useWindowVisibility", () => ({
  useWindowVisibility: () => ({ current: true })
}));

function setup(length: number, base: number, selected = 0, keyboardMode = false) {
  let keyHandler: (event: KeyboardEvent) => Promise<void>;
  vi.stubGlobal("window", {
    addEventListener: (name: string, handler: typeof keyHandler) => {
      if (name === "keydown") keyHandler = handler;
    },
    removeEventListener: () => {}
  });
  const history = Array.from({ length }, (_, index) => ({
    id: index + 1, content: `item ${index}`, content_type: "text"
  }) as ClipboardEntry);
  const copyToClipboard = vi.fn().mockResolvedValue(undefined);
  const render = () => useKeyboardNavigation({
    filteredHistory: history,
    selectionBaseIndex: base,
    selectedIndex: selected,
    setSelectedIndex: (value) => { selected = typeof value === "function" ? value(selected) : value; },
    isKeyboardMode: keyboardMode,
    setIsKeyboardMode: (value) => { keyboardMode = typeof value === "function" ? value(keyboardMode) : value; },
    showSettings: false,
    showTagManager: false,
    chatMode: false,
    editingTagsId: null,
    arrowKeySelection: true,
    richPasteHotkey: "F9",
    searchInputRef: { current: null },
    copyToClipboard,
    setSearch: () => {}
  });
  render();
  return {
    copyToClipboard,
    selected: () => selected,
    key: async (key: string) => {
      await keyHandler({
        key, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
        target: { tagName: "DIV", classList: { contains: () => false } },
        preventDefault: () => {}, stopPropagation: () => {}
      } as unknown as KeyboardEvent);
      render();
    },
    action: async (payload: string) => {
      await native.handler!({ payload });
      render();
    }
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllTimers();
  vi.useRealTimers();
  native.handler = undefined;
});

describe("collapsed pinned keyboard selection", () => {
  it.each(["key", "action"] as const)("%s cannot select or paste when every item is hidden", async (path) => {
    const nav = setup(2, 2);
    for (const direction of ["down", "up", "down"]) {
      await nav[path](path === "key" ? (direction === "down" ? "ArrowDown" : "ArrowUp") : direction);
      expect(nav.selected()).toBe(-1);
    }
    await nav[path](path === "key" ? "Enter" : "enter");
    expect(nav.copyToClipboard).not.toHaveBeenCalled();
  });

  it.each(["key", "action"] as const)("%s keeps an empty list unselected", async (path) => {
    const nav = setup(0, 0);
    await nav[path](path === "key" ? "ArrowDown" : "down");
    expect(nav.selected()).toBe(-1);
    await nav[path](path === "key" ? "Enter" : "enter");
    expect(nav.copyToClipboard).not.toHaveBeenCalled();
  });

  it.each(["Enter", "F9", "enter"])("%s rejects a stale selection below the visible range", async (key) => {
    const nav = setup(3, 2, 0, true);
    if (key === "enter") await nav.action(key);
    else await nav.key(key);
    expect(nav.copyToClipboard).not.toHaveBeenCalled();
  });

  it.each(["key", "action"] as const)("%s navigates and pastes visible items after the pinned block", async (path) => {
    vi.useFakeTimers();
    const nav = setup(4, 2);
    const send = (key: string, action: string) => nav[path](path === "key" ? key : action);
    await send("ArrowDown", "down");
    expect(nav.selected()).toBe(2);
    await send("ArrowUp", "up");
    expect(nav.selected()).toBe(2);
    await send("ArrowDown", "down");
    await send("ArrowDown", "down");
    expect(nav.selected()).toBe(3);
    await send("Enter", "enter");
    expect(nav.copyToClipboard).toHaveBeenCalledWith(4, "item 3", "text", false);
  });

  it("selects the first pinned item when the section is expanded", async () => {
    const nav = setup(2, 0, -1);
    await nav.key("ArrowDown");
    expect(nav.selected()).toBe(0);
  });
});
