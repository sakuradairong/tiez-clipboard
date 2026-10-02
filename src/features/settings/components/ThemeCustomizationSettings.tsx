import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
    title: "自定义界面", accent: "强调色", radius: "圆角", inherit: "跟随主题",
    apply: "应用外观", reset: "恢复主题默认", preview: "条目预览", sample: "周六上午出发，先去书店。",
    presets: "个人预设", name: "预设名称", save: "保存当前预设", load: "应用预设", remove: "删除预设",
    placeholder: "我的界面", choose: "选择已保存的预设", saved: "外观已保存", presetSaved: "预设已保存",
    failed: "保存失败", nameRequired: "请输入预设名称", limit: "最多保存 20 个预设", saving: "保存中…"
  },
  tw: {
    title: "自訂介面", accent: "強調色", radius: "圓角", inherit: "跟隨主題",
    apply: "套用外觀", reset: "恢復主題預設", preview: "項目預覽", sample: "週六上午出發，先去書店。",
    presets: "個人預設", name: "預設名稱", save: "儲存目前預設", load: "套用預設", remove: "刪除預設",
    placeholder: "我的介面", choose: "選擇已儲存的預設", saved: "外觀已儲存", presetSaved: "預設已儲存",
    failed: "儲存失敗", nameRequired: "請輸入預設名稱", limit: "最多儲存 20 個預設", saving: "儲存中…"
  },
  en: {
    title: "Customize appearance", accent: "Accent color", radius: "Corners", inherit: "Use theme default",
    apply: "Apply appearance", reset: "Restore theme defaults", preview: "Item preview", sample: "Leave on Saturday morning. Stop by the bookstore first.",
    presets: "Personal presets", name: "Preset name", save: "Save current preset", load: "Apply preset", remove: "Delete preset",
    placeholder: "My appearance", choose: "Choose a saved preset", saved: "Appearance saved", presetSaved: "Preset saved",
    failed: "Save failed", nameRequired: "Enter a preset name", limit: "Save up to 20 presets", saving: "Saving…"
  }
};

const ThemeCustomizationSettings = ({ language, profile, presetsJson }: ThemeCustomizationSettingsProps) => {
  const text = labels[language];
  const [draft, setDraft] = useState<ThemeCustomization>(profile.customization);
  const [name, setName] = useState("");
  const [selectedPreset, setSelectedPreset] = useState("");
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
    await persist({
      ...appearanceProfileToSettings(preset.profile),
      [APPEARANCE_PRESETS_SETTING]: JSON.stringify(next)
    }, text.presetSaved);
  };

  const chosen = presets.find((preset) => preset.id === selectedPreset);

  return (
    <div className="appearance-customization">
      <h4>{text.title}</h4>
      <div className="appearance-customization-fields">
        <div className="appearance-customization-field">
          <label htmlFor="appearance-accent">{text.accent}</label>
          <div className="appearance-customization-row">
            <input
              id="appearance-accent"
              type="color"
              value={draft.accentColor ?? "#2f6fed"}
              disabled={busy}
              onInput={(event) => {
                const accentColor = event.currentTarget.value;
                setDraft((previous) => ({ ...previous, accentColor }));
              }}
            />
            <span>{draft.accentColor ?? text.inherit}</span>
          </div>
        </div>
        <div className="appearance-customization-field">
          <label htmlFor="appearance-radius">{text.radius}: {draft.cornerRadius === null ? text.inherit : `${draft.cornerRadius}px`}</label>
          <input
            id="appearance-radius"
            type="range"
            min="0"
            max="24"
            step="1"
            value={draft.cornerRadius ?? 12}
            disabled={busy}
            onChange={(event) => setDraft((previous) => ({ ...previous, cornerRadius: Number(event.target.value) }))}
          />
        </div>
      </div>
      <div className="appearance-item-preview" aria-label={text.preview}>
        <div className="appearance-preview-selected">{text.sample}</div>
        <div>https://v2.tauri.app/</div>
      </div>
      <div className="appearance-customization-actions">
        <button className="btn-icon" type="button" disabled={busy} onClick={() => void persist({ [THEME_CUSTOMIZATION_SETTING]: JSON.stringify(draft) }, text.saved)}>
          {busy ? text.saving : text.apply}
        </button>
        <button className="btn-icon" type="button" disabled={busy} onClick={() => {
          const defaults: ThemeCustomization = { accentColor: null, cornerRadius: null };
          setDraft(defaults);
          void persist({ [THEME_CUSTOMIZATION_SETTING]: JSON.stringify(defaults) }, text.saved);
        }}>{text.reset}</button>
      </div>
      <h4>{text.presets}</h4>
      <label className="appearance-customization-field" htmlFor="appearance-preset-name">
        {text.name}
        <input id="appearance-preset-name" className="search-input" maxLength={60} value={name} placeholder={text.placeholder} disabled={busy} onChange={(event) => setName(event.target.value)} />
      </label>
      <div className="appearance-customization-actions">
        <button className="btn-icon" type="button" disabled={busy} onClick={() => void savePreset()}>{text.save}</button>
      </div>
      {presets.length > 0 && (
        <>
          <select aria-label={text.presets} value={selectedPreset} disabled={busy} onChange={(event) => setSelectedPreset(event.target.value)}>
            <option value="">{text.choose}</option>
            {presets.map((preset) => <option key={preset.id} value={preset.id}>{preset.name} · {getThemeLabel(preset.profile.theme, language)}</option>)}
          </select>
          <div className="appearance-customization-actions">
            <button className="btn-icon" type="button" disabled={busy || !chosen} onClick={() => {
              if (chosen) void persist(appearanceProfileToSettings(chosen.profile), text.saved).then((success) => {
                if (success) setDraft(chosen.profile.customization);
              });
            }}>{text.load}</button>
            <button className="btn-icon" type="button" disabled={busy || !chosen} onClick={() => {
              if (chosen) void persist({ [APPEARANCE_PRESETS_SETTING]: JSON.stringify(presets.filter((preset) => preset.id !== chosen.id)) }, text.presetSaved);
            }}>{text.remove}</button>
          </div>
        </>
      )}
      {feedback && <div className={`appearance-customization-feedback ${failed ? "error" : ""}`} role={failed ? "alert" : "status"}>{feedback}</div>}
    </div>
  );
};

export default ThemeCustomizationSettings;
