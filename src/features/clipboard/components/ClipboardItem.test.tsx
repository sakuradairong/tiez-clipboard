import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import type { ClipboardItemProps } from "../types";
import type { ClipboardEntry } from "../../../shared/types";
import ClipboardItem from "./ClipboardItem";

type EffectSlot = { deps?: unknown[]; cleanup?: () => void };
const hooks = vi.hoisted(() => ({
  slots: [] as unknown[], cursor: 0, effects: [] as Array<() => void>,
  compare: undefined as ((a: ClipboardItemProps, b: ClipboardItemProps) => boolean) | undefined
}));
vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const effect = (callback: () => void | (() => void), deps?: unknown[]) => {
    const index = hooks.cursor++;
    const previous = hooks.slots[index] as EffectSlot | undefined;
    if (previous?.deps && deps && deps.every((dep, i) => Object.is(dep, previous.deps![i]))) return;
    const slot: EffectSlot = { deps };
    hooks.slots[index] = slot;
    hooks.effects.push(() => { previous?.cleanup?.(); slot.cleanup = callback() || undefined; });
  };
  return {
    ...actual,
    memo: (component: unknown, compare: typeof hooks.compare) => {
      hooks.compare = compare;
      return component;
    },
    useRef: (current: unknown) => hooks.slots[hooks.cursor++] ??
      (hooks.slots[hooks.cursor - 1] = { current }),
    useState: (initial: unknown) => {
      const index = hooks.cursor++;
      if (!(index in hooks.slots)) hooks.slots[index] = typeof initial === "function" ? initial() : initial;
      return [hooks.slots[index], (value: unknown) => {
        hooks.slots[index] = typeof value === "function" ? value(hooks.slots[index]) : value;
      }];
    },
    useMemo: (callback: () => unknown) => { hooks.cursor++; return callback(); },
    useEffect: effect,
    useLayoutEffect: effect
  };
});
const native = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => Promise<void> | void>(),
  preview: {
    emit: vi.fn().mockResolvedValue(undefined), show: vi.fn().mockResolvedValue(undefined),
    hide: vi.fn().mockResolvedValue(undefined), setPosition: vi.fn().mockResolvedValue(undefined),
    setAlwaysOnTop: vi.fn().mockResolvedValue(undefined),
    setIgnoreCursorEvents: vi.fn().mockResolvedValue(undefined),
    isVisible: vi.fn().mockResolvedValue(false)
  },
  scaleFactor: vi.fn().mockResolvedValue(1)
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined), convertFileSrc: (v: string) => v }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, callback: (event: { payload: unknown }) => void) => {
    native.listeners.set(name, callback);
    return Promise.resolve(() => {});
  }
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  WebviewWindow: { getByLabel: async () => native.preview }
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    scaleFactor: native.scaleFactor, outerPosition: async () => ({ x: 0, y: 0 }),
    outerSize: async () => ({ width: 350, height: 380 })
  }),
  currentMonitor: async () => ({ position: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } }),
  PhysicalPosition: class {}, PhysicalSize: class {}
}));
vi.mock("framer-motion", () => ({ motion: { div: "div" }, AnimatePresence: "div" }));
vi.mock("../../../shared/components/HtmlContent", () => ({ default: "div" }));

const render = (props: ClipboardItemProps & { compactMode?: boolean }) => {
  hooks.cursor = 0;
  const element = (ClipboardItem as unknown as (value: ClipboardItemProps & { compactMode?: boolean }) => ReactElement<{
    ref: { current: unknown }; onMouseEnter: (event: unknown) => void
  }>)(props);
  element.props.ref.current = { isConnected: true, matches: () => true };
  hooks.effects.splice(0).forEach((effect) => effect());
  return element;
};

function props(hidden: boolean): ClipboardItemProps & { compactMode: boolean } {
  return {
    item: { id: 1, content: "sensitive secret", preview: "sensitive secret", content_type: "text", tags: ["Password"] } as ClipboardEntry,
    isSelected: false, windowPinned: false, isSensitiveHidden: hidden, isRevealed: !hidden,
    isEditingTags: false, tagInput: "", theme: "retro", language: "zh", t: (key) => key,
    compactMode: true, onSelect: vi.fn(), onCopy: vi.fn(), onToggleReveal: vi.fn(), onOpen: vi.fn(),
    onTogglePin: vi.fn(), onDelete: vi.fn(), onToggleTagEditor: vi.fn(), onTagInput: vi.fn(),
    onTagAdd: vi.fn(), onTagDelete: vi.fn()
  };
}

const hover = (element: ReturnType<typeof render>) => element.props.onMouseEnter({
  clientX: 10, clientY: 10, screenX: 10, screenY: 10,
  currentTarget: { isConnected: true }
});

beforeEach(() => {
  hooks.slots = [];
  hooks.cursor = 0;
  hooks.effects = [];
  vi.clearAllMocks();
  native.scaleFactor.mockResolvedValue(1);
  vi.useFakeTimers();
  vi.stubGlobal("document", { documentElement: { classList: { contains: () => false } } });
  vi.stubGlobal("getComputedStyle", () => ({ getPropertyValue: () => "13" }));
});

afterEach(async () => {
  hooks.slots.forEach((slot) => (slot as EffectSlot)?.cleanup?.());
  await Promise.resolve();
  await Promise.resolve();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("compact preview privacy", () => {
  it("never sends concealed content after hovering", async () => {
    hover(render(props(true)));
    await vi.advanceTimersByTimeAsync(1500);
    expect(native.preview.emit.mock.calls.some(([event]) => event === "compact-preview-update")).toBe(false);
    expect(native.preview.show).not.toHaveBeenCalled();
  });

  it("cancels the pending hover when the item becomes hidden", async () => {
    const visible = props(false);
    hover(render(visible));
    render({ ...visible, isSensitiveHidden: true, isRevealed: false });
    await vi.advanceTimersByTimeAsync(1500);
    expect(native.preview.emit.mock.calls.some(([event]) => event === "compact-preview-update")).toBe(false);
  });

  it("clears emitted content and rejects a native show still awaiting its position", async () => {
    const visible = props(false);
    hover(render(visible));
    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(native.preview.emit).toHaveBeenCalledWith(
      "compact-preview-update", expect.objectContaining({ content: "sensitive secret" })
    ));
    let releaseScale!: (scale: number) => void;
    native.scaleFactor.mockReturnValueOnce(new Promise((resolve) => { releaseScale = resolve; }));
    const pending = native.listeners.get("compact-preview-resize")!({ payload: { width: 320, height: 220 } });
    render({ ...visible, isSensitiveHidden: true, isRevealed: false });
    releaseScale(1);
    await pending;
    await Promise.resolve();
    expect(native.preview.emit).toHaveBeenCalledWith("compact-preview-clear");
    expect(native.preview.hide).toHaveBeenCalled();
    expect(native.preview.show).not.toHaveBeenCalled();
  });

  it("invalidates memoization when concealment or mask settings change", () => {
    const visible = props(false);
    expect(hooks.compare!(visible, { ...visible, isSensitiveHidden: true })).toBe(false);
    expect(hooks.compare!(visible, { ...visible, sensitiveMaskPrefixVisible: 0 })).toBe(false);
    expect(hooks.compare!(visible, { ...visible, sensitiveMaskSuffixVisible: 0 })).toBe(false);
    expect(hooks.compare!(visible, { ...visible, sensitiveMaskEmailDomain: true })).toBe(false);
  });

  it("closes and clears an already visible preview when concealment changes", async () => {
    const visible = props(false);
    hover(render(visible));
    await vi.advanceTimersByTimeAsync(1200);
    await vi.waitFor(() => expect(native.preview.show).toHaveBeenCalled());
    native.preview.hide.mockClear();
    native.preview.emit.mockClear();
    render({ ...visible, isSensitiveHidden: true, isRevealed: false });
    await Promise.resolve();
    await Promise.resolve();
    expect(native.preview.emit).toHaveBeenCalledWith("compact-preview-clear");
    expect(native.preview.hide).toHaveBeenCalled();
  });
});
