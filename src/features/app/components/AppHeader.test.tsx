import { describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import AppHeader from "./AppHeader";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined)
}));

function findElement(node: unknown, predicate: (el: ReactElement<any>) => boolean): ReactElement<any> | null {
  if (!node || typeof node !== "object") return null;
  const el = node as ReactElement<any>;
  if (predicate(el)) return el;
  const children = el.props?.children;
  if (Array.isArray(children)) {
    for (const child of children) {
      const found = findElement(child, predicate);
      if (found) return found;
    }
  } else if (children) {
    return findElement(children, predicate);
  }
  return null;
}

describe("AppHeader IME composition and shortcut protection", () => {
  const createProps = (overrides = {}) => ({
    t: (key: string) => key,
    showSettings: false,
    setShowSettings: vi.fn(),
    showTagManager: false,
    setShowTagManager: vi.fn(),
    tagManagerEnabled: true,
    showEmojiPanel: false,
    setShowEmojiPanel: vi.fn(),
    emojiPanelEnabled: true,
    chatMode: false,
    fileServerEnabled: false,
    isWindowPinned: false,
    setIsWindowPinned: vi.fn(),
    clearHistory: vi.fn(),
    showSearchBox: true,
    search: "test query",
    setSearch: vi.fn(),
    isComposing: false,
    setIsComposing: vi.fn(),
    searchInputRef: { current: null },
    showTagFilter: false,
    setShowTagFilter: vi.fn(),
    allTags: [],
    searchIsFocused: true,
    setSearchIsFocused: vi.fn(),
    setEditingTagsId: vi.fn(),
    theme: "minimal",
    colorMode: "dark",
    settingsTitle: "Settings",
    typeFilter: null,
    setTypeFilter: vi.fn(),
    onBack: vi.fn(),
    onToggleChat: vi.fn(),
    ...overrides
  });

  it("does not clear search on Escape when isComposing prop is true", () => {
    const setSearch = vi.fn();
    const props = createProps({ search: "query", isComposing: true, setSearch });
    const tree = AppHeader(props);
    const input = findElement(tree, (el) => el.type === "input");
    expect(input).not.toBeNull();

    const stopPropagation = vi.fn();
    input!.props.onKeyDown({
      key: "Escape",
      keyCode: 27,
      nativeEvent: { isComposing: false },
      stopPropagation
    });

    expect(setSearch).not.toHaveBeenCalled();
    expect(stopPropagation).not.toHaveBeenCalled();
  });

  it("does not clear search on Escape when nativeEvent.isComposing is true", () => {
    const setSearch = vi.fn();
    const props = createProps({ search: "query", isComposing: false, setSearch });
    const tree = AppHeader(props);
    const input = findElement(tree, (el) => el.type === "input");
    expect(input).not.toBeNull();

    const stopPropagation = vi.fn();
    input!.props.onKeyDown({
      key: "Escape",
      keyCode: 27,
      nativeEvent: { isComposing: true },
      stopPropagation
    });

    expect(setSearch).not.toHaveBeenCalled();
    expect(stopPropagation).not.toHaveBeenCalled();
  });

  it("does not clear search on Escape when keyCode is 229 (IME processing)", () => {
    const setSearch = vi.fn();
    const props = createProps({ search: "query", isComposing: false, setSearch });
    const tree = AppHeader(props);
    const input = findElement(tree, (el) => el.type === "input");
    expect(input).not.toBeNull();

    const stopPropagation = vi.fn();
    input!.props.onKeyDown({
      key: "Escape",
      keyCode: 229,
      nativeEvent: { isComposing: false },
      stopPropagation
    });

    expect(setSearch).not.toHaveBeenCalled();
    expect(stopPropagation).not.toHaveBeenCalled();
  });

  it("clears search and stops propagation on normal Escape when not composing", () => {
    const setSearch = vi.fn();
    const props = createProps({ search: "query", isComposing: false, setSearch });
    const tree = AppHeader(props);
    const input = findElement(tree, (el) => el.type === "input");
    expect(input).not.toBeNull();

    const stopPropagation = vi.fn();
    input!.props.onKeyDown({
      key: "Escape",
      keyCode: 27,
      nativeEvent: { isComposing: false },
      stopPropagation
    });

    expect(stopPropagation).toHaveBeenCalled();
    expect(setSearch).toHaveBeenCalledWith("");
  });
});
