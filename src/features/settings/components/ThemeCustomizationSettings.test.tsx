import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import ThemeCustomizationSettings from "./ThemeCustomizationSettings";
import type { AppearanceProfile } from "../../../shared/lib/appearance";

type EffectSlot = { deps?: unknown[]; cleanup?: () => void };
const hooks = vi.hoisted(() => ({
  slots: [] as unknown[],
  cursor: 0,
  effects: [] as Array<() => void>
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
    useRef: (current: unknown) => {
      const idx = hooks.cursor++;
      if (!(idx in hooks.slots)) {
        hooks.slots[idx] = { current };
      }
      return hooks.slots[idx];
    },
    useState: (initial: unknown) => {
      const index = hooks.cursor++;
      if (!(index in hooks.slots)) {
        hooks.slots[index] = typeof initial === "function" ? initial() : initial;
      }
      const setState = (value: unknown) => {
        hooks.slots[index] = typeof value === "function" ? value(hooks.slots[index]) : value;
      };
      return [hooks.slots[index], setState];
    },
    useMemo: (callback: () => unknown) => { hooks.cursor++; return callback(); },
    useEffect: effect,
    useLayoutEffect: effect
  };
});

const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args)
}));

function findAllElements(node: unknown, predicate: (el: ReactElement<any>) => boolean): Array<ReactElement<any>> {
  const results: Array<ReactElement<any>> = [];
  function walk(n: unknown) {
    if (!n || typeof n !== "object") return;
    if (Array.isArray(n)) {
      n.forEach(walk);
      return;
    }
    const el = n as ReactElement<any>;
    if (predicate(el)) results.push(el);
    const children = el.props?.children;
    if (children) {
      walk(children);
    }
  }
  walk(node);
  return results;
}

function findElement(node: unknown, predicate: (el: ReactElement<any>) => boolean): ReactElement<any> | null {
  const matches = findAllElements(node, predicate);
  return matches.length > 0 ? matches[0] : null;
}

const defaultProfile: AppearanceProfile = {
  theme: "minimal",
  colorMode: "dark",
  compactMode: false,
  clipboardItemFontSize: 13,
  clipboardTagFontSize: 10,
  surfaceOpacity: 50,
  customBackground: "",
  customBackgroundOpacity: 45,
  customization: { accentColor: "#2563eb", cornerRadius: 6 }
};

describe("ThemeCustomizationSettings IME composition and busy state protection", () => {
  beforeEach(() => {
    hooks.slots = [];
    hooks.cursor = 0;
    hooks.effects = [];
    vi.clearAllMocks();
    invokeMock.mockResolvedValue(undefined);
  });

  afterEach(() => {
    hooks.slots.forEach((slot) => (slot as EffectSlot)?.cleanup?.());
    vi.clearAllMocks();
  });

  const renderComponent = (busy = false, name = "My Theme") => {
    hooks.cursor = 0;
    // slot 0: draft
    // slot 1: name
    // slot 2: isComposing
    // slot 3: busy
    // slot 4: feedback
    // slot 5: failed
    if (hooks.slots.length === 0) {
      hooks.slots[0] = defaultProfile.customization;
      hooks.slots[1] = name;
      hooks.slots[2] = false;
      hooks.slots[3] = busy;
      hooks.slots[4] = "";
      hooks.slots[5] = false;
    } else {
      hooks.slots[1] = name;
      hooks.slots[3] = busy;
    }
    const tree = ThemeCustomizationSettings({
      language: "zh",
      profile: defaultProfile,
      presetsJson: "[]"
    });
    hooks.effects.splice(0).forEach((effect) => effect());
    return tree;
  };

  it("does not save preset on Enter when nativeEvent.isComposing is true", async () => {
    const tree = renderComponent(false, "Preset Test");
    const input = findElement(tree, (el) => el.props?.id === "appearance-preset-name");
    expect(input).not.toBeNull();

    const preventDefault = vi.fn();
    input!.props.onKeyDown({
      key: "Enter",
      keyCode: 13,
      nativeEvent: { isComposing: true },
      preventDefault
    });

    expect(invokeMock).not.toHaveBeenCalled();
    expect(preventDefault).not.toHaveBeenCalled();
  });

  it("does not save preset on Enter when keyCode is 229 (IME processing)", async () => {
    const tree = renderComponent(false, "Preset Test");
    const input = findElement(tree, (el) => el.props?.id === "appearance-preset-name");
    expect(input).not.toBeNull();

    const preventDefault = vi.fn();
    input!.props.onKeyDown({
      key: "Enter",
      keyCode: 229,
      nativeEvent: { isComposing: false },
      preventDefault
    });

    expect(invokeMock).not.toHaveBeenCalled();
    expect(preventDefault).not.toHaveBeenCalled();
  });

  it("does not save preset on Enter when composition is active (via onCompositionStart)", async () => {
    const tree = renderComponent(false, "Preset Test");
    const input = findElement(tree, (el) => el.props?.id === "appearance-preset-name");
    expect(input).not.toBeNull();

    // Start composition
    input!.props.onCompositionStart();

    // Re-render to reflect state change
    const updatedTree = renderComponent(false, "Preset Test");
    const updatedInput = findElement(updatedTree, (el) => el.props?.id === "appearance-preset-name");

    const preventDefault = vi.fn();
    updatedInput!.props.onKeyDown({
      key: "Enter",
      keyCode: 13,
      nativeEvent: { isComposing: false },
      preventDefault
    });

    expect(invokeMock).not.toHaveBeenCalled();
    expect(preventDefault).not.toHaveBeenCalled();

    // End composition
    updatedInput!.props.onCompositionEnd();
    const finalTree = renderComponent(false, "Preset Test");
    const finalInput = findElement(finalTree, (el) => el.props?.id === "appearance-preset-name");

    finalInput!.props.onKeyDown({
      key: "Enter",
      keyCode: 13,
      nativeEvent: { isComposing: false },
      preventDefault
    });

    expect(preventDefault).toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith("save_appearance_settings", expect.any(Object));
  });

  it("does not save preset on Enter when busy is true", () => {
    const tree = renderComponent(true, "Preset Test");
    const input = findElement(tree, (el) => el.props?.id === "appearance-preset-name");
    expect(input).not.toBeNull();

    const preventDefault = vi.fn();
    input!.props.onKeyDown({
      key: "Enter",
      keyCode: 13,
      nativeEvent: { isComposing: false },
      preventDefault
    });

    expect(invokeMock).not.toHaveBeenCalled();
    expect(preventDefault).not.toHaveBeenCalled();
  });

  it("disables all appearance draft controls and preset buttons while busy", () => {
    const tree = renderComponent(true, "Preset Test");

    // Palette swatches
    const swatches = findAllElements(tree, (el) =>
      typeof el.props?.className === "string" && el.props.className.includes("appearance-palette-swatch")
    );
    expect(swatches.length).toBeGreaterThan(0);
    swatches.forEach((swatch) => {
      expect(swatch.props.disabled).toBe(true);
    });

    // Custom color input
    const colorInput = findElement(tree, (el) => el.props?.id === "appearance-accent");
    expect(colorInput).not.toBeNull();
    expect(colorInput!.props.disabled).toBe(true);

    // Radius slider
    const radiusSlider = findElement(tree, (el) => el.props?.id === "appearance-radius");
    expect(radiusSlider).not.toBeNull();
    expect(radiusSlider!.props.disabled).toBe(true);

    // Quick radius chips
    const chips = findAllElements(tree, (el) =>
      typeof el.props?.className === "string" && el.props.className.includes("appearance-chip")
    );
    expect(chips.length).toBeGreaterThan(0);
    chips.forEach((chip) => {
      expect(chip.props.disabled).toBe(true);
    });

    // Reset buttons (inherit theme)
    const resetButtons = findAllElements(tree, (el) =>
      typeof el.props?.className === "string" && el.props.className.includes("appearance-field-reset-btn")
    );
    expect(resetButtons.length).toBe(2);
    resetButtons.forEach((btn) => {
      expect(btn.props.disabled).toBe(true);
    });

    // Apply and Reset defaults buttons
    const applyBtn = findElement(tree, (el) =>
      typeof el.props?.className === "string" && el.props.className.includes("appearance-btn-primary")
    );
    expect(applyBtn).not.toBeNull();
    expect(applyBtn!.props.disabled).toBe(true);

    const resetDefaultsBtn = findElement(tree, (el) =>
      typeof el.props?.className === "string" && el.props.className.includes("appearance-btn-secondary")
    );
    expect(resetDefaultsBtn).not.toBeNull();
    expect(resetDefaultsBtn!.props.disabled).toBe(true);

    // Preset name input and save button
    const presetInput = findElement(tree, (el) => el.props?.id === "appearance-preset-name");
    expect(presetInput!.props.disabled).toBe(true);

    const presetSaveBtn = findElement(tree, (el) =>
      typeof el.props?.className === "string" && el.props.className.includes("appearance-preset-save-btn")
    );
    expect(presetSaveBtn!.props.disabled).toBe(true);
  });
});
