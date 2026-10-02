import { DEFAULT_THEME, normalizeThemeId } from "../config/themes";

export const THEME_CUSTOMIZATION_SETTING = "app.theme_customization";
export const APPEARANCE_PRESETS_SETTING = "app.appearance_presets";

export interface ThemeCustomization {
  accentColor: string | null;
  cornerRadius: number | null;
}

export interface AppearanceProfile {
  theme: string;
  colorMode: "system" | "light" | "dark";
  compactMode: boolean;
  clipboardItemFontSize: number;
  clipboardTagFontSize: number;
  surfaceOpacity: number;
  customBackground: string;
  customBackgroundOpacity: number;
  customization: ThemeCustomization;
}

export interface AppearancePreset {
  id: string;
  name: string;
  profile: AppearanceProfile;
}

const objectValue = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};

const parseJSON = (raw?: string | null): unknown => {
  if (!raw || raw.length > 262144) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
};

const clampNumber = (value: unknown, fallback: number, min: number, max: number): number =>
  typeof value === "number" && Number.isFinite(value)
    ? Math.min(max, Math.max(min, value))
    : fallback;

const normalizeCustomization = (value: unknown): ThemeCustomization => {
  const source = objectValue(value);
  return {
    accentColor: typeof source.accentColor === "string" && /^#[0-9a-f]{6}$/i.test(source.accentColor)
      ? source.accentColor.toLowerCase()
      : null,
    cornerRadius: typeof source.cornerRadius === "number" && Number.isFinite(source.cornerRadius)
      ? clampNumber(source.cornerRadius, 0, 0, 24)
      : null,
  };
};

export const parseThemeCustomization = (raw?: string | null): ThemeCustomization =>
  normalizeCustomization(parseJSON(raw));

const normalizeProfile = (value: unknown): AppearanceProfile => {
  const source = objectValue(value);
  const theme = typeof source.theme === "string" && /^[a-zA-Z0-9_-]{1,128}$/.test(source.theme)
    && (!source.theme.startsWith("store-") || source.theme.length > 6)
    ? normalizeThemeId(source.theme)
    : DEFAULT_THEME;
  return {
    theme,
    colorMode: source.colorMode === "light" || source.colorMode === "dark" ? source.colorMode : "system",
    compactMode: source.compactMode === true,
    clipboardItemFontSize: Math.round(clampNumber(source.clipboardItemFontSize, 13, 11, 18)),
    clipboardTagFontSize: Math.round(clampNumber(source.clipboardTagFontSize, 10, 8, 14)),
    surfaceOpacity: Math.round(clampNumber(source.surfaceOpacity, 50, 0, 100)),
    customBackground: typeof source.customBackground === "string" && source.customBackground.length <= 4096
      && !/[\u0000-\u001f]/.test(source.customBackground) ? source.customBackground : "",
    customBackgroundOpacity: Math.round(clampNumber(source.customBackgroundOpacity, 45, 0, 100)),
    customization: normalizeCustomization(source.customization),
  };
};

export const parseAppearancePresets = (raw?: string | null): AppearancePreset[] => {
  const parsed = parseJSON(raw);
  if (!Array.isArray(parsed)) return [];
  const result: AppearancePreset[] = [];
  const ids = new Set<string>();
  for (const value of parsed) {
    const source = objectValue(value);
    if (typeof source.id !== "string" || !/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$/.test(source.id)
      || ids.has(source.id) || typeof source.name !== "string" || !source.name.trim()
      || source.profile === null || typeof source.profile !== "object" || Array.isArray(source.profile)) continue;
    ids.add(source.id);
    result.push({ id: source.id, name: source.name.trim().slice(0, 60), profile: normalizeProfile(source.profile) });
    if (result.length === 20) break;
  }
  return result;
};

const settingNumber = (value?: string): number | undefined => {
  if (!value?.trim()) return undefined;
  const number = Number(value);
  return Number.isFinite(number) ? number : undefined;
};

export const appearanceProfileFromSettings = (settings: Record<string, string>): AppearanceProfile =>
  normalizeProfile({
    theme: settings["app.theme"],
    colorMode: settings["app.color_mode"],
    compactMode: settings["app.compact_mode"] === "true",
    clipboardItemFontSize: settingNumber(settings["app.clipboard_item_font_size"]),
    clipboardTagFontSize: settingNumber(settings["app.clipboard_tag_font_size"]),
    surfaceOpacity: settingNumber(settings["app.surface_opacity"]),
    customBackground: settings["app.custom_background"],
    customBackgroundOpacity: settingNumber(settings["app.custom_background_opacity"]),
    customization: parseThemeCustomization(settings[THEME_CUSTOMIZATION_SETTING]),
  });

export const appearanceProfileToSettings = (profile: AppearanceProfile): Record<string, string> => {
  const normalized = normalizeProfile(profile);
  return {
    "app.theme": normalized.theme,
    "app.color_mode": normalized.colorMode,
    "app.compact_mode": String(normalized.compactMode),
    "app.clipboard_item_font_size": String(normalized.clipboardItemFontSize),
    "app.clipboard_tag_font_size": String(normalized.clipboardTagFontSize),
    "app.surface_opacity": String(normalized.surfaceOpacity),
    "app.custom_background": normalized.customBackground,
    "app.custom_background_opacity": String(normalized.customBackgroundOpacity),
    [THEME_CUSTOMIZATION_SETTING]: JSON.stringify(normalized.customization),
  };
};

const radiusTokens = [
  "--radius-window", "--radius-panel", "--radius-card", "--radius-control",
  "--shell-radius", "--title-radius", "--input-radius", "--button-radius", "--card-radius",
  "--panel-radius", "--data-panel-radius", "--keycap-radius", "--segmented-radius", "--segment-radius",
  "--modal-radius", "--dialog-button-radius", "--tags-radius", "--tag-chip-radius", "--menu-radius",
  "--queue-radius", "--reset-button-radius",
];

interface InlineToken {
  value: string;
  priority: string;
}

const previousTokens = new WeakMap<Element, Map<string, InlineToken>>();

// Inline overrides live on both theme targets; removing them reveals the active theme's tokens.
export const applyThemeCustomization = (customization: ThemeCustomization, targets?: Element[]): void => {
  const normalized = normalizeCustomization(customization);
  const tokens: Record<string, string> = {};
  if (normalized.accentColor) {
    const color = normalized.accentColor;
    const channels = [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16));
    const rgb = channels.join(", ");
    const linear = channels.map((channel) => {
      const value = channel / 255;
      return value <= 0.04045 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
    });
    const luminance = linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
    const contrast = (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) ? "#000000" : "#ffffff";
    const hover = `#${channels.map((channel) => Math.round(channel * 0.85).toString(16).padStart(2, "0")).join("")}`;
    Object.assign(tokens, {
      "--accent-color": color,
      "--accent-color-rgb": rgb,
      "--accent-hover": hover,
      "--accent-soft": `rgba(${rgb}, 0.12)`,
      "--accent-contrast-color": contrast,
      "--input-focus-border-color": color,
      "--button-active-filled-background": color,
      "--button-active-filled-color": contrast,
      "--button-active-filled-border-color": color,
      "--card-selected-background": `rgba(${rgb}, 0.08)`,
      "--card-selected-border-color": `rgba(${rgb}, 0.5)`,
      "--switch-track-background": `rgba(${rgb}, 0.15)`,
      "--switch-track-border": `1px solid rgba(${rgb}, 0.16)`,
      "--switch-track-active-background": color,
      "--switch-track-active-border-color": color,
      "--switch-thumb-background": contrast,
      "--segment-active-background": color,
      "--segment-active-color": contrast,
      "--menu-item-hover-background": color,
      "--menu-item-hover-color": contrast,
    });
  }
  if (normalized.cornerRadius !== null) {
    for (const token of radiusTokens) tokens[token] = `${normalized.cornerRadius}px`;
  }
  const themeTargets = targets ?? (typeof document === "undefined" ? [] : [document.documentElement, document.body]);
  for (const target of new Set(themeTargets)) {
    const style = (target as HTMLElement).style;
    if (!style) continue;
    const previous = previousTokens.get(target);
    previous?.forEach(({ value, priority }, token) => {
      if (value) style.setProperty(token, value, priority);
      else style.removeProperty(token);
    });
    previousTokens.delete(target);
    const saved = new Map<string, InlineToken>();
    for (const [token, value] of Object.entries(tokens)) {
      saved.set(token, { value: style.getPropertyValue(token), priority: style.getPropertyPriority(token) });
      style.setProperty(token, value);
    }
    if (saved.size) previousTokens.set(target, saved);
  }
};
