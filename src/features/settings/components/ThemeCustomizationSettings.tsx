import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Sparkles, Trash2, RotateCcw, Save } from "lucide-react";
import type { Locale } from "../../../shared/types";
import {
  APPEARANCE_PRESETS_SETTING,
  THEME_CUSTOMIZATION_SETTING,
  applyThemeCustomization,
  appearanceProfileFromSettings,
  appearanceProfileToSettings,
  parseAppearancePresets,
  type AppearanceProfile,
  type ThemeCustomization
} from "../../../shared/lib/appearance";
import { getThemeLabel } from "../../../shared/config/themes";

interface ThemeCustomizationSettingsProps {
  language: Locale;
  profile: AppearanceProfile;
  presetsJson?: string;
}

const labels = {
  zh: {
    title: "外观自定义", accent: "强调色", radius: "圆角大小", inherit: "跟随主题",
    apply: "应用外观", reset: "恢复主题默认", preview: "效果实时预览",
    presets: "个人外观预设", name: "预设名称", save: "保存当前", load: "应用", remove: "删除",
    placeholder: "输入预设名称，如：极简深蓝", choose: "选择已保存的预设", saved: "外观设置已生效并保存",
    presetSaved: "预设已成功保存", failed: "保存失败", nameRequired: "请输入预设名称",
    limit: "最多保存 20 个预设", saving: "保存中…", customColor: "自定义色值",
    noPresets: "暂无保存的预设，可将当前配置保存为快捷预设",
    sampleCode: "const tiez = '极简、高效的桌面剪贴板';",
    quickPalette: "推荐色板", quickCorners: "预设圆角"
  },
  tw: {
    title: "外觀自訂", accent: "強調色", radius: "圓角大小", inherit: "跟隨主題",
    apply: "套用外觀", reset: "恢復主題預設", preview: "效果即時預覽",
    presets: "個人外觀預設", name: "預設名稱", save: "儲存目前", load: "套用", remove: "刪除",
    placeholder: "輸入預設名稱，如：極簡深藍", choose: "選擇已儲存的預設", saved: "外觀設定已生效並儲存",
    presetSaved: "預設已成功儲存", failed: "儲存失敗", nameRequired: "請輸入預設名稱",
    limit: "最多儲存 20 個預設", saving: "儲存中…", customColor: "自訂色值",
    noPresets: "暫無儲存的預設，可將目前配置儲存為快捷預設",
    sampleCode: "const tiez = '極簡、高效的桌面剪貼簿';",
    quickPalette: "推薦色盤", quickCorners: "預設圓角"
  },
  en: {
    title: "Appearance Tuning", accent: "Accent Color", radius: "Corner Radius", inherit: "Theme Default",
    apply: "Apply Appearance", reset: "Restore Defaults", preview: "Live Preview",
    presets: "Personal Presets", name: "Preset Name", save: "Save Current", load: "Apply", remove: "Delete",
    placeholder: "Preset name, e.g. Minimal Blue", choose: "Choose a saved preset", saved: "Appearance saved",
    presetSaved: "Preset saved", failed: "Save failed", nameRequired: "Enter a preset name",
    limit: "Up to 20 presets allowed", saving: "Saving…", customColor: "Custom hex",
    noPresets: "No saved presets yet. Save your current look for quick switching.",
    sampleCode: "const tiez = 'A fast, elegant desktop clipboard';",
    quickPalette: "Palette", quickCorners: "Presets"
  }
};

const ACCENT_PALETTE = [
  { name: "Blue", color: "#2563eb" },
  { name: "Indigo", color: "#6366f1" },
  { name: "Emerald", color: "#059669" },
  { name: "Violet", color: "#8b5cf6" },
  { name: "Rose", color: "#e11d48" },
  { name: "Amber", color: "#d97706" }
];

const RADIUS_OPTIONS = [
  { label: "0px", value: 0 },
  { label: "6px", value: 6 },
  { label: "12px", value: 12 },
  { label: "18px", value: 18 }
];

const ThemeCustomizationSettings = ({ language, profile, presetsJson }: ThemeCustomizationSettingsProps) => {
  const text = labels[language] || labels.zh;
  const [draft, setDraft] = useState<ThemeCustomization>(profile.customization);
  const [name, setName] = useState("");
  const [isComposing, setIsComposing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [failed, setFailed] = useState(false);
  const presets = parseAppearancePresets(presetsJson);
  const committed = useRef(profile.customization);
  committed.current = profile.customization;

  useEffect(() => {
    setDraft(profile.customization);
  }, [profile.customization.accentColor, profile.customization.cornerRadius, profile.theme]);

  // Preview only while this settings section is open; leaving discards unsaved edits.
  useLayoutEffect(() => {
    applyThemeCustomization(draft);
    return () => applyThemeCustomization(committed.current);
  }, [draft, profile.theme]);

  const persist = async (settings: Record<string, string>, success: string) => {
    setBusy(true);
    setFailed(false);
    setFeedback("");
    try {
      await invoke("save_appearance_settings", { settings });
      setFeedback(success);
      return true;
    } catch (error) {
      setFailed(true);
      setFeedback(`${text.failed}: ${error instanceof Error ? error.message : String(error)}`);
      return false;
    } finally {
      setBusy(false);
    }
  };

  const savePreset = async () => {
    if (busy) return;
    const presetName = name.trim();
    if (!presetName) {
      setFailed(true);
      setFeedback(text.nameRequired);
      return;
    }
    const existing = presets.find((preset) => preset.name === presetName);
    if (!existing && presets.length >= 20) {
      setFailed(true);
      setFeedback(text.limit);
      return;
    }
    const currentProfile = appearanceProfileFromSettings(
      appearanceProfileToSettings({ ...profile, customization: draft })
    );
    const preset = {
      id: existing?.id ?? crypto.randomUUID(),
      name: presetName,
      profile: currentProfile
    };
    const next = [...presets.filter((item) => item.id !== preset.id), preset];
    const success = await persist({
      ...appearanceProfileToSettings(preset.profile),
      [APPEARANCE_PRESETS_SETTING]: JSON.stringify(next)
    }, text.presetSaved);
    if (success) {
      setName("");
    }
  };

  return (
    <div className="appearance-customization">
      {/* Real-time Customization Group */}
      <div className="appearance-customization-section">
        <div className="appearance-section-header">
          <h4>{text.title}</h4>
          <span className="appearance-section-badge">
            {draft.accentColor ? draft.accentColor.toUpperCase() : text.inherit}
            {" · "}
            {draft.cornerRadius !== null ? `${draft.cornerRadius}px` : text.inherit}
          </span>
        </div>

        {/* Accent Color Tuning */}
        <div className="appearance-customization-field">
          <div className="appearance-field-title-row">
            <label htmlFor="appearance-accent">{text.accent}</label>
            <button
              type="button"
              className="appearance-field-reset-btn"
              disabled={busy}
              onClick={() => setDraft((prev) => ({ ...prev, accentColor: null }))}
              title={text.inherit}
            >
              {text.inherit}
            </button>
          </div>

          <div className="appearance-palette-row">
            {ACCENT_PALETTE.map((item) => {
              const isSelected = (draft.accentColor || "").toLowerCase() === item.color.toLowerCase();
              return (
                <button
                  key={item.color}
                  type="button"
                  disabled={busy}
                  className={`appearance-palette-swatch ${isSelected ? "selected" : ""}`}
                  style={{ backgroundColor: item.color }}
                  onClick={() => setDraft((prev) => ({ ...prev, accentColor: item.color }))}
                  title={item.name}
                >
                  {isSelected && <Check size={12} color="#ffffff" />}
                </button>
              );
            })}
            <div className="appearance-custom-color-wrapper">
              <input
                id="appearance-accent"
                type="color"
                className="appearance-color-input"
                value={draft.accentColor ?? "#2563eb"}
                disabled={busy}
                onInput={(event) => {
                  const accentColor = event.currentTarget.value;
                  setDraft((prev) => ({ ...prev, accentColor }));
                }}
                title={text.customColor}
              />
            </div>
          </div>
        </div>

        {/* Corner Radius Tuning */}
        <div className="appearance-customization-field">
          <div className="appearance-field-title-row">
            <label htmlFor="appearance-radius">
              {text.radius}: {draft.cornerRadius === null ? text.inherit : `${draft.cornerRadius}px`}
            </label>
            <button
              type="button"
              className="appearance-field-reset-btn"
              disabled={busy}
              onClick={() => setDraft((prev) => ({ ...prev, cornerRadius: null }))}
              title={text.inherit}
            >
              {text.inherit}
            </button>
          </div>

          <div className="appearance-radius-controls">
            <input
              id="appearance-radius"
              type="range"
              min="0"
              max="24"
              step="1"
              className="appearance-radius-slider"
              value={draft.cornerRadius ?? 10}
              disabled={busy}
              onChange={(event) => setDraft((prev) => ({ ...prev, cornerRadius: Number(event.target.value) }))}
            />
            <div className="appearance-quick-radius-chips">
              {RADIUS_OPTIONS.map((opt) => (
                <button
                  key={opt.value}
                  type="button"
                  disabled={busy}
                  className={`appearance-chip ${draft.cornerRadius === opt.value ? "active" : ""}`}
                  onClick={() => setDraft((prev) => ({ ...prev, cornerRadius: opt.value }))}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Live Mock Clipboard Item Preview */}
        <div className="appearance-preview-container">
          <span className="appearance-preview-label">{text.preview}</span>
          <div
            className="appearance-mock-card"
            style={{
              borderRadius: draft.cornerRadius !== null ? `${draft.cornerRadius}px` : undefined,
              borderColor: draft.accentColor ? draft.accentColor : undefined
            }}
          >
            <div className="appearance-mock-meta">
              <span className="appearance-mock-app">VS Code</span>
              <span className="appearance-mock-time">刚刚</span>
            </div>
            <div className="appearance-mock-content">
              {text.sampleCode}
            </div>
            <div className="appearance-mock-footer">
              <span
                className="appearance-mock-tag"
                style={{
                  backgroundColor: draft.accentColor ? draft.accentColor : "var(--accent-color)"
                }}
              >
                Code
              </span>
            </div>
          </div>
        </div>

        {/* Action Buttons */}
        <div className="appearance-customization-actions">
          <button
            className="btn-icon appearance-btn-primary"
            type="button"
            disabled={busy}
            onClick={() => void persist({ [THEME_CUSTOMIZATION_SETTING]: JSON.stringify(draft) }, text.saved)}
          >
            <Sparkles size={13} style={{ marginRight: 4 }} />
            {busy ? text.saving : text.apply}
          </button>
          <button
            className="btn-icon appearance-btn-secondary"
            type="button"
            disabled={busy}
            onClick={() => {
              const defaults: ThemeCustomization = { accentColor: null, cornerRadius: null };
              setDraft(defaults);
              void persist({ [THEME_CUSTOMIZATION_SETTING]: JSON.stringify(defaults) }, text.saved);
            }}
          >
            <RotateCcw size={13} style={{ marginRight: 4 }} />
            {text.reset}
          </button>
        </div>
      </div>

      {/* Personal Presets Group */}
      <div className="appearance-customization-section">
        <div className="appearance-section-header">
          <h4>{text.presets}</h4>
          <span className="appearance-section-badge">{presets.length}/20</span>
        </div>

        <div className="appearance-preset-save-row">
          <input
            id="appearance-preset-name"
            className="search-input appearance-preset-input"
            maxLength={60}
            value={name}
            placeholder={text.placeholder}
            disabled={busy}
            onChange={(event) => setName(event.target.value)}
            onCompositionStart={() => setIsComposing(true)}
            onCompositionEnd={() => setIsComposing(false)}
            onKeyDown={(e) => {
              if (isComposing || e.nativeEvent.isComposing || e.keyCode === 229) return;
              if (e.key === "Enter" && !busy && name.trim()) {
                e.preventDefault();
                void savePreset();
              }
            }}
          />
          <button
            className="btn-icon appearance-preset-save-btn"
            type="button"
            disabled={busy || !name.trim()}
            onClick={() => void savePreset()}
            title={text.save}
          >
            <Save size={13} style={{ marginRight: 3 }} />
            {text.save}
          </button>
        </div>

        {/* Preset Cards List */}
        {presets.length > 0 ? (
          <div className="appearance-presets-list">
            {presets.map((preset) => (
              <div key={preset.id} className="appearance-preset-item">
                <div className="appearance-preset-info">
                  <span
                    className="appearance-preset-dot"
                    style={{
                      backgroundColor: preset.profile.customization.accentColor || "var(--accent-color)"
                    }}
                  />
                  <span className="appearance-preset-name">{preset.name}</span>
                  <span className="appearance-preset-theme">
                    {getThemeLabel(preset.profile.theme, language)}
                  </span>
                </div>
                <div className="appearance-preset-actions">
                  <button
                    className="btn-icon appearance-preset-apply-btn"
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      void persist(appearanceProfileToSettings(preset.profile), text.saved).then((success) => {
                        if (success) setDraft(preset.profile.customization);
                      });
                    }}
                    title={text.load}
                  >
                    {text.load}
                  </button>
                  <button
                    className="btn-icon appearance-preset-delete-btn"
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      void persist({
                        [APPEARANCE_PRESETS_SETTING]: JSON.stringify(presets.filter((item) => item.id !== preset.id))
                      }, text.presetSaved);
                    }}
                    title={text.remove}
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="appearance-presets-empty">{text.noPresets}</div>
        )}
      </div>

      {feedback && (
        <div
          className={`appearance-customization-feedback ${failed ? "error" : "success"}`}
          role={failed ? "alert" : "status"}
        >
          {feedback}
        </div>
      )}
    </div>
  );
};

export default ThemeCustomizationSettings;
