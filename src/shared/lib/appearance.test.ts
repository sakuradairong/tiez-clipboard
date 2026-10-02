import { describe, it, expect, vi } from "vitest";
import {
  APPEARANCE_PRESETS_SETTING,
  THEME_CUSTOMIZATION_SETTING,
  appearanceProfileFromSettings,
  appearanceProfileToSettings,
  applyThemeCustomization,
  parseAppearancePresets,
  parseThemeCustomization,
} from "./appearance";

const emptyCustomization = { accentColor: null, cornerRadius: null };

function styleTarget() {
  const values = new Map<string, { value: string; priority: string }>();
  const style = {
    setProperty: (name: string, value: string, priority = "") => { values.set(name, { value, priority }); },
    removeProperty: (name: string) => { const value = values.get(name)?.value ?? ""; values.delete(name); return value; },
    getPropertyValue: (name: string) => values.get(name)?.value ?? "",
    getPropertyPriority: (name: string) => values.get(name)?.priority ?? "",
  };
  return { target: { style } as unknown as Element, style, values };
}

describe("theme customization validation", () => {
  it("uses theme defaults for malformed JSON, arrays, null and CSS injection", () => {
    for (const raw of [undefined, "", "{", "null", "[]", "42", '{"accentColor":"red; background:url(https://example.com)","cornerRadius":"12px"}']) {
      expect(parseThemeCustomization(raw)).toEqual(emptyCustomization);
    }
    expect(parseThemeCustomization('{"__proto__":{"accentColor":"#ffffff"},"css":"body{display:none}"}'))
      .toEqual(emptyCustomization);
    expect({}).not.toHaveProperty("accentColor");
  });

  it("accepts only six-digit HEX colors and finite numeric radii", () => {
    expect(parseThemeCustomization('{"accentColor":"#Ab12Cd","cornerRadius":7.5}'))
      .toEqual({ accentColor: "#ab12cd", cornerRadius: 7.5 });
    for (const color of ["#fff", "#ffffffff", "rgb(1,2,3)", " #ffffff", "#fffffg"]) {
      expect(parseThemeCustomization(JSON.stringify({ accentColor: color })).accentColor).toBeNull();
    }
    expect(parseThemeCustomization('{"cornerRadius":-4}').cornerRadius).toBe(0);
    expect(parseThemeCustomization('{"cornerRadius":400}').cornerRadius).toBe(24);
    expect(parseThemeCustomization('{"cornerRadius":1e400}').cornerRadius).toBeNull();
  });
});

describe("theme customization tokens", () => {
  it("keeps RGB, filled controls and contrasting text consistent on both targets", () => {
    const root = styleTarget();
    const body = styleTarget();
    applyThemeCustomization({ accentColor: "#fff5cc", cornerRadius: 0 }, [root.target, body.target]);
    for (const target of [root, body]) {
      expect(target.style.getPropertyValue("--accent-color")).toBe("#fff5cc");
      expect(target.style.getPropertyValue("--accent-color-rgb")).toBe("255, 245, 204");
      expect(target.style.getPropertyValue("--accent-soft")).toBe("rgba(255, 245, 204, 0.12)");
      expect(target.style.getPropertyValue("--accent-hover")).toMatch(/^#[a-f0-9]{6}$/);
      expect(target.style.getPropertyValue("--accent-contrast-color")).toBe("#000000");
      for (const token of ["--button-active-filled-color", "--segment-active-color", "--menu-item-hover-color"]) {
        expect(target.style.getPropertyValue(token)).toBe("#000000");
      }
      for (const token of ["--button-active-filled-background", "--segment-active-background", "--menu-item-hover-background"]) {
        expect(target.style.getPropertyValue(token)).toBe("#fff5cc");
      }
      for (const token of ["--card-radius", "--input-radius", "--panel-radius", "--modal-radius"]) {
        expect(target.style.getPropertyValue(token)).toBe("0px");
      }
    }
    applyThemeCustomization({ accentColor: "#001144", cornerRadius: null }, [root.target, body.target]);
    expect(root.style.getPropertyValue("--button-active-filled-color")).toBe("#ffffff");
    expect(body.style.getPropertyValue("--card-radius")).toBe("");
    expect(root.style.getPropertyValue("--card-selected-shadow")).toBe("");
    expect(root.style.getPropertyValue("--card-selected-outline")).toBe("");
  });

  it("clears only its overrides and restores existing inline values and priority", () => {
    const target = styleTarget();
    target.style.setProperty("--accent-color", "#123456", "important");
    target.style.setProperty("--unrelated-token", "unchanged");
    applyThemeCustomization({ accentColor: "#ffffff", cornerRadius: 20 }, [target.target]);
    applyThemeCustomization({ accentColor: "#000000", cornerRadius: 6 }, [target.target]);
    applyThemeCustomization(emptyCustomization, [target.target]);
    expect(target.style.getPropertyValue("--accent-color")).toBe("#123456");
    expect(target.style.getPropertyPriority("--accent-color")).toBe("important");
    expect(target.style.getPropertyValue("--unrelated-token")).toBe("unchanged");
    expect(target.values.size).toBe(2);
    applyThemeCustomization(emptyCustomization, [target.target]);
    expect(target.values.size).toBe(2);
  });

  it("defaults to html and body without requiring a DOM test dependency", () => {
    const root = styleTarget();
    const body = styleTarget();
    vi.stubGlobal("document", { documentElement: root.target, body: body.target });
    try {
      applyThemeCustomization({ accentColor: "#4466ee", cornerRadius: 12 });
      expect(root.style.getPropertyValue("--card-radius")).toBe("12px");
      expect(body.style.getPropertyValue("--card-radius")).toBe("12px");
      applyThemeCustomization(emptyCustomization);
      expect(root.values.size).toBe(0);
      expect(body.values.size).toBe(0);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("appearance profiles and presets", () => {
  it("preserves an appearance profile through the nine settings keys and presets", () => {
    const profile = {
      theme: "store-community-42",
      colorMode: "dark" as const,
      compactMode: true,
      clipboardItemFontSize: 15,
      clipboardTagFontSize: 11,
      surfaceOpacity: 62,
      customBackground: "C:\\Pictures\\wallpaper.png",
      customBackgroundOpacity: 37,
      customization: { accentColor: "#4455ee", cornerRadius: 8 },
    };
    const settings = appearanceProfileToSettings(profile);
    expect(Object.keys(settings)).toHaveLength(9);
    expect(settings[THEME_CUSTOMIZATION_SETTING]).toBe(JSON.stringify(profile.customization));
    expect(settings).not.toHaveProperty(APPEARANCE_PRESETS_SETTING);
    expect(appearanceProfileFromSettings(settings)).toEqual(profile);
    expect(parseAppearancePresets(JSON.stringify([{ id: "my-preset", name: "我的外观", profile }])))
      .toEqual([{ id: "my-preset", name: "我的外观", profile }]);
  });

  it("clamps profile ranges, normalizes unknown themes and rejects invalid setting numbers", () => {
    const defaults = appearanceProfileFromSettings({});
    expect(defaults).toEqual({
      theme: "mica", colorMode: "system", compactMode: false,
      clipboardItemFontSize: 13, clipboardTagFontSize: 10, surfaceOpacity: 50,
      customBackground: "", customBackgroundOpacity: 45, customization: emptyCustomization,
    });
    expect(appearanceProfileFromSettings({
      "app.theme": "unknown-theme", "app.color_mode": "unexpected", "app.compact_mode": "false",
      "app.clipboard_item_font_size": "999", "app.clipboard_tag_font_size": "-1",
      "app.surface_opacity": "Infinity", "app.custom_background_opacity": " ",
    })).toEqual({ ...defaults, clipboardItemFontSize: 18, clipboardTagFontSize: 8 });
    const [preset] = parseAppearancePresets(JSON.stringify([{ id: "p", name: " P ", profile: {
      theme: "store-invalid theme", compactMode: "true", surfaceOpacity: -50,
      customBackgroundOpacity: 140, customBackground: "bad\u0000path",
      customization: { accentColor: "#ffffff", cornerRadius: 100 },
    } }]));
    expect(preset.name).toBe("P");
    expect(preset.profile).toEqual({ ...defaults, surfaceOpacity: 0, customBackgroundOpacity: 100,
      customization: { accentColor: "#ffffff", cornerRadius: 24 } });
    const rounded = appearanceProfileFromSettings({
      "app.clipboard_item_font_size": "14.7", "app.clipboard_tag_font_size": "10.2",
      "app.surface_opacity": "49.8", "app.custom_background_opacity": "33.4",
    });
    expect(rounded.clipboardItemFontSize).toBe(15);
    expect(rounded.clipboardTagFontSize).toBe(10);
    expect(rounded.surfaceOpacity).toBe(50);
    expect(rounded.customBackgroundOpacity).toBe(33);
    expect(appearanceProfileFromSettings({ "app.theme": "store-" }).theme).toBe("mica");
  });

  it("ignores arbitrary fields, broken presets and duplicate IDs, with a 20-preset cap", () => {
    for (const raw of [undefined, "{", "null", "{}", '"text"']) expect(parseAppearancePresets(raw)).toEqual([]);
    const profile = appearanceProfileFromSettings({});
    const malicious = JSON.parse('{"id":"valid","name":"Saved","profile":{"__proto__":{"theme":"retro"},"css":"body{display:none}"},"script":"alert(1)"}');
    const presets = parseAppearancePresets(JSON.stringify([
      null, { id: "bad id", name: "Bad", profile }, { id: "empty", name: " ", profile },
      { id: "missing", name: "Missing" }, malicious, malicious,
      ...Array.from({ length: 30 }, (_, index) => ({ id: `p-${index}`, name: `Preset ${index}`, profile })),
    ]));
    expect(presets).toHaveLength(20);
    expect(presets[0]).toEqual({ id: "valid", name: "Saved", profile });
    expect(presets[0]).not.toHaveProperty("script");
    expect(presets[0].profile).not.toHaveProperty("css");
    expect(new Set(presets.map((preset) => preset.id)).size).toBe(20);
  });
});
