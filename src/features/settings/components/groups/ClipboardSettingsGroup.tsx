import { useEffect, useRef, useState } from "react";
import type { ComponentType, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronRight } from "lucide-react";
import { getHotkeyDisplayTokens } from "../../../../shared/lib/hotkeyDisplay";
import { isMacPlatform, isWindowsPlatform } from "../../../../shared/lib/platform";
import type { QuickPasteModifier } from "../../../app/types";
import type { HotkeyMode } from "../../../../shared/hooks/useHotkeyConfig";

interface LabelWithHintProps {
    label: string;
    hint?: string | ReactNode;
    hintKey: string;
}

interface ClipboardSettingsGroupProps {
    t: (key: string) => string;
    collapsed: boolean;
    onToggle: () => void;
    LabelWithHint: ComponentType<LabelWithHintProps>;
    relayAvailable: boolean;
    persistent: boolean;
    setPersistent: (val: boolean) => void;
    persistentLimitEnabled: boolean;
    setPersistentLimitEnabled: (val: boolean) => void;
    persistentLimit: number;
    setPersistentLimit: (val: number) => void;
    saveAppSetting: (key: string, val: string) => void;
    pushToast: (msg: string, duration?: number) => unknown;
    deduplicate: boolean;
    setDeduplicate: (val: boolean) => void;
    captureFiles: boolean;
    setCaptureFiles: (val: boolean) => void;
    captureRichText: boolean;
    setCaptureRichText: (val: boolean) => void;
    richTextSnapshotPreview: boolean;
    setRichTextSnapshotPreview: (val: boolean) => void;
    richPasteHotkey: string;
    isRecordingRich: boolean;
    setIsRecordingRich: (val: boolean) => void;
    updateRichPasteHotkey: (key: string) => void;
    plainPasteHotkey: string;
    isRecordingPlain: boolean;
    setIsRecordingPlain: (val: boolean) => void;
    updatePlainPasteHotkey: (key: string) => void;
    searchHotkey: string;
    isRecordingSearch: boolean;
    setIsRecordingSearch: (val: boolean) => void;
    updateSearchHotkey: (key: string) => void;
    relaySendHotkey: string;
    isRecordingRelaySend: boolean;
    setIsRecordingRelaySend: (val: boolean) => void;
    updateRelaySendHotkey: (key: string) => void;
    relayFetchHotkey: string;
    isRecordingRelayFetch: boolean;
    setIsRecordingRelayFetch: (val: boolean) => void;
    updateRelayFetchHotkey: (key: string) => void;
    quickPasteModifier: QuickPasteModifier;
    setQuickPasteModifier: (val: QuickPasteModifier) => void;
    deleteAfterPaste: boolean;
    setDeleteAfterPaste: (val: boolean) => void;
    moveToTopAfterPaste: boolean;
    setMoveToTopAfterPaste: (val: boolean) => void;
    sequentialMode: boolean;
    setSequentialModeState: (val: boolean) => void;
    sequentialHotkey: string;
    isRecordingSequential: boolean;
    setIsRecordingSequential: (val: boolean) => void;
    updateSequentialHotkey: (key: string) => void;
    checkHotkeyConflict: (newHotkey: string, mode: HotkeyMode) => boolean;
    privacyProtection: boolean;
    setPrivacyProtection: (val: boolean) => void;
    privacyProtectionKinds: string[];
    setPrivacyProtectionKinds: (val: string[]) => void;
    privacyProtectionCustomRules: string;
    setPrivacyProtectionCustomRules: (val: string) => void;
    sensitiveMaskPrefixVisible: number;
    setSensitiveMaskPrefixVisible: (val: number) => void;
    sensitiveMaskSuffixVisible: number;
    setSensitiveMaskSuffixVisible: (val: number) => void;
    sensitiveMaskEmailDomain: boolean;
    setSensitiveMaskEmailDomain: (val: boolean) => void;
    privacyKindsOpen: boolean;
    setPrivacyKindsOpen: (val: boolean) => void;
    privacyRulesOpen: boolean;
    setPrivacyRulesOpen: (val: boolean) => void;
    isRecording: boolean;
    setIsRecording: (val: boolean) => void;
    hotkeyParts: string[];
    updateHotkey: (key: string) => Promise<boolean>;
    hotkey: string;
    registryWinVEnabled: boolean;
    setRegistryWinVEnabled: (val: boolean) => void;
    appSettings: Record<string, string>;
    theme: string;
    colorMode: string;
}

const ClipboardSettingsGroup = (props: ClipboardSettingsGroupProps) => {
    const isWindows = isWindowsPlatform();
    const stopAllRecording = () => {
        props.setIsRecording(false);
        props.setIsRecordingSequential(false);
        props.setIsRecordingRich(false);
        props.setIsRecordingPlain(false);
        props.setIsRecordingSearch(false);
        props.setIsRecordingRelaySend(false);
        props.setIsRecordingRelayFetch(false);
    };
    const beginRecording = (mode: HotkeyMode) => {
        stopAllRecording();
        if (mode === "main") props.setIsRecording(true);
        if (mode === "sequential") props.setIsRecordingSequential(true);
        if (mode === "rich") props.setIsRecordingRich(true);
        if (mode === "plain") props.setIsRecordingPlain(true);
        if (mode === "search") props.setIsRecordingSearch(true);
        if (mode === "relaySend") props.setIsRecordingRelaySend(true);
        if (mode === "relayFetch") props.setIsRecordingRelayFetch(true);
    };
    const beginRelayRecording = (mode: "send" | "fetch") => {
        beginRecording(mode === "send" ? "relaySend" : "relayFetch");
    };
    const quickPasteOptions: Array<{ value: QuickPasteModifier; label: string }> = isMacPlatform()
        ? [
            { value: "disabled", label: props.t("quick_paste_modifier_disabled") },
            { value: "ctrl", label: "Control (⌃)" },
            { value: "alt", label: "Option (⌥)" },
            { value: "shift", label: "Shift (⇧)" },
            { value: "win", label: "Command (⌘)" }
        ]
        : [
            { value: "disabled", label: props.t("quick_paste_modifier_disabled") },
            { value: "ctrl", label: props.t("quick_paste_modifier_ctrl") },
            { value: "alt", label: props.t("quick_paste_modifier_alt") },
            { value: "shift", label: props.t("quick_paste_modifier_shift") },
            { value: "win", label: props.t("quick_paste_modifier_win") }
        ];
    const [persistentLimitDraft, setPersistentLimitDraft] = useState(
        props.persistentLimit.toString()
    );
    const [maskSettingsOpen, setMaskSettingsOpen] = useState(false);
    const [winVUpdating, setWinVUpdating] = useState(false);
    const winVUpdatingRef = useRef(false);
    const preWinVHotkeyRef = useRef(props.appSettings['app.pre_win_v_hotkey'] || 'Alt+C');

    useEffect(() => {
        setPersistentLimitDraft(props.persistentLimit.toString());
    }, [props.persistentLimit, props.persistentLimitEnabled]);

    useEffect(() => {
        const savedHotkey = props.appSettings['app.pre_win_v_hotkey'];
        if (savedHotkey && savedHotkey !== 'Win+V') {
            preWinVHotkeyRef.current = savedHotkey;
        }
    }, [props.appSettings]);

    const restartExplorerIfConfirmed = async (changed: boolean): Promise<boolean> => {
        if (!changed) return false;

        const confirmed = await ask(
            props.t('restart_explorer_confirm'),
            { title: props.t('restart_explorer_title'), kind: 'warning' }
        );
        if (!confirmed) return false;

        await invoke('restart_explorer');
        await new Promise((resolve) => window.setTimeout(resolve, 1500));
        return true;
    };

    const updateWinVShortcut = async (enabled: boolean) => {
        if (winVUpdatingRef.current || enabled === props.registryWinVEnabled) return;

        winVUpdatingRef.current = true;
        setWinVUpdating(true);
        const previousHotkey = props.hotkey;
        let registryChanged = false;
        let explorerRestarted = false;
        let hotkeyChanged = false;

        try {
            if (enabled) {
                if (previousHotkey && previousHotkey !== 'Win+V') {
                    await invoke('save_setting', {
                        key: 'app.pre_win_v_hotkey',
                        value: previousHotkey
                    });
                    preWinVHotkeyRef.current = previousHotkey;
                }

                registryChanged = await invoke<boolean>('trigger_registry_win_v_optimization', {
                    enable: true
                });
                explorerRestarted = await restartExplorerIfConfirmed(registryChanged);
                if (registryChanged && !explorerRestarted) {
                    throw new Error(props.t('win_v_restart_cancelled'));
                }
                if (previousHotkey !== 'Win+V' && !(await props.updateHotkey('Win+V'))) {
                    throw new Error(props.t('hotkey_register_failed'));
                }
                hotkeyChanged = previousHotkey !== 'Win+V';
            } else {
                const fallbackHotkey = preWinVHotkeyRef.current !== 'Win+V'
                    ? preWinVHotkeyRef.current
                    : 'Alt+C';
                if (!(await props.updateHotkey(fallbackHotkey))) {
                    throw new Error(props.t('hotkey_register_failed'));
                }
                hotkeyChanged = fallbackHotkey !== previousHotkey;

                registryChanged = await invoke<boolean>('trigger_registry_win_v_optimization', {
                    enable: false
                });
                explorerRestarted = await restartExplorerIfConfirmed(registryChanged);
                if (registryChanged && !explorerRestarted) {
                    throw new Error(props.t('win_v_restart_cancelled'));
                }
            }

            await invoke('save_setting', {
                key: 'app.use_win_v_shortcut',
                value: String(enabled)
            });
            props.setRegistryWinVEnabled(enabled);
            props.pushToast(props.t(enabled ? 'win_v_enabled_msg' : 'win_v_disabled_msg'), 3000);
        } catch (error) {
            console.error('Failed to update Win+V shortcut:', error);

            let rollbackError: unknown;
            try {
                const rollbackChanged = await invoke<boolean>('trigger_registry_win_v_optimization', {
                    enable: !enabled
                });
                if (explorerRestarted && rollbackChanged) {
                    await invoke('restart_explorer');
                }
            } catch (registryRollbackError) {
                rollbackError = registryRollbackError;
                console.error('Failed to roll back the Win+V registry state:', registryRollbackError);
            }

            if (hotkeyChanged) {
                try {
                    if (!(await props.updateHotkey(previousHotkey))) {
                        throw new Error(props.t('hotkey_register_failed'));
                    }
                } catch (hotkeyRollbackError) {
                    rollbackError ??= hotkeyRollbackError;
                    console.error('Failed to roll back the Win+V hotkey:', hotkeyRollbackError);
                }
            }

            if (!rollbackError) {
                props.setRegistryWinVEnabled(!enabled);
            }
            const details = rollbackError ? `${String(error)}; ${String(rollbackError)}` : String(error);
            props.pushToast(`${props.t('win_v_update_failed')}${details}`, 5000);
        } finally {
            winVUpdatingRef.current = false;
            setWinVUpdating(false);
        }
    };

    const commitPersistentLimit = (rawValue?: string) => {
        const source = rawValue ?? persistentLimitDraft;
        const parsed = parseInt(source, 10);
        if (!Number.isFinite(parsed)) {
            setPersistentLimitDraft(props.persistentLimit.toString());
            return;
        }
        const clamped = Math.max(50, Math.min(99999, parsed));
        props.setPersistentLimit(clamped);
        props.saveAppSetting('persistent_limit', clamped.toString());
        if (clamped.toString() !== source) {
            setPersistentLimitDraft(clamped.toString());
        }
    };

    const renderHotkeyCaps = (hotkey: string) => {
        const tokens = getHotkeyDisplayTokens(hotkey);
        if (tokens.length === 0) {
            return <div className="key-cap" style={{ width: '8em', opacity: 0.5 }}>{props.t('not_set')}</div>;
        }
        const compactLabel = tokens.map((token) => token.label).join("");
        return <div className="key-cap key-cap-chord">{compactLabel}</div>;
    };

    return (
        <div className={`settings-group ${props.collapsed ? 'collapsed' : ''}`}>
            <div className="group-header" onClick={props.onToggle}>
                <h3 style={{ margin: 0 }}>{props.t('clipboard_settings')}</h3>
                {props.collapsed ? <ChevronRight size={16} /> : <ChevronDown size={16} />}
            </div>
            {!props.collapsed && (
                <div className="group-content">
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('persistent_storage')}
                            hint={props.t('persistent_hint')}
                            hintKey="persistent_storage"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.persistent}
                                onChange={(e) => props.setPersistent(e.target.checked)}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>
                    {props.persistent && (
                        <>
                            <div className="setting-item">
                                <props.LabelWithHint
                                    label={props.t('persistent_limit_enabled')}
                                    hint={props.t('persistent_limit_enabled_hint')}
                                    hintKey="persistent_limit_enabled"
                                />
                                <label className="switch">
                                    <input
                                        className="cb"
                                        type="checkbox"
                                        checked={props.persistentLimitEnabled}
                                        onChange={(e) => {
                                            props.setPersistentLimitEnabled(e.target.checked);
                                            props.saveAppSetting('persistent_limit_enabled', e.target.checked.toString());
                                        }}
                                    />
                                    <div className="toggle"><div className="left" /><div className="right" /></div>
                                </label>
                            </div>
                            {props.persistentLimitEnabled && (
                                <div className="setting-item">
                                    <props.LabelWithHint
                                        label={props.t('persistent_limit')}
                                        hint={props.t('persistent_limit_hint')}
                                        hintKey="persistent_limit"
                                    />
                                    <input
                                        type="number"
                                        value={persistentLimitDraft}
                                        onFocus={(e) => {
                                            e.target.select();
                                            invoke("focus_clipboard_window").catch(console.error);
                                        }}
                                        onChange={(e) => {
                                            const next = e.target.value;
                                            if (next === "") {
                                                setPersistentLimitDraft("");
                                                return;
                                            }
                                            if (!/^\d+$/.test(next)) return;
                                            setPersistentLimitDraft(next);
                                        }}
                                        onBlur={() => {
                                            commitPersistentLimit();
                                        }}
                                        onKeyDown={(e) => {
                                            if (e.key === 'Enter') {
                                                commitPersistentLimit(e.currentTarget.value);
                                                e.currentTarget.blur();
                                            }
                                        }}
                                        style={{
                                            width: '90px',
                                            padding: '4px 8px',
                                            borderRadius: '4px',
                                            border: '1px solid var(--border-color)',
                                            background: 'var(--input-bg)',
                                            color: 'var(--text-color)',
                                            fontSize: '14px'
                                        }}
                                    />
                                </div>
                            )}
                        </>
                    )}
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('merge_duplicates')}
                            hint={props.t('merge_duplicates_hint') || "Time limit to prevent accidental multiple copies"}
                            hintKey="merge_duplicates"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.deduplicate}
                                onChange={(e) => props.setDeduplicate(e.target.checked)}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>
                    <div className="setting-item">
                        <div className="item-label-group">
                            <span className="item-label">{props.t('capture_files')}</span>
                        </div>
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.captureFiles}
                                onChange={(e) => props.setCaptureFiles(e.target.checked)}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('capture_rich_text') || '捕获富文本'}
                            hint={props.t('capture_rich_text_hint') || '开启后可记录富文本并支持双击带格式粘贴'}
                            hintKey="capture_rich_text"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.captureRichText}
                                onChange={(e) => {
                                    const val = e.target.checked;
                                    props.setCaptureRichText(val);
                                }}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('rich_text_snapshot_preview') || '富文本快照预览'}
                            hint={props.t('rich_text_snapshot_preview_hint') || '开启后将富文本转换为内存快照图用于条目与悬浮预览'}
                            hintKey="rich_text_snapshot_preview"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.richTextSnapshotPreview}
                                onChange={(e) => {
                                    const val = e.target.checked;
                                    props.setRichTextSnapshotPreview(val);
                                    props.saveAppSetting('rich_text_snapshot_preview', String(val));
                                }}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>


                    <div className="setting-item">
                        <div className="item-label-group">
                            <span className="item-label">{props.t('rich_paste_hotkey_label')}</span>
                            <span className="hint">{props.isRecordingRich ? props.t('hotkey_recording_esc') : props.t('hotkey_click_hint')}</span>
                        </div>
                        <div
                            className={`key-group ${props.isRecordingRich ? 'recording' : ''}`}
                            onClick={(e) => { beginRecording("rich"); e.currentTarget.focus(); }}
                            tabIndex={0}
                            onKeyDown={(e) => {
                                if (!props.isRecordingRich) return;
                                e.preventDefault();
                                e.stopPropagation();

                                if (e.key === 'Escape') {
                                    props.setIsRecordingRich(false);
                                    return;
                                }

                                if (e.key === 'Backspace' || e.key === 'Delete') {
                                    props.updateRichPasteHotkey('');
                                    props.setIsRecordingRich(false);
                                    return;
                                }

                                const modifiers = [];
                                if (e.ctrlKey) modifiers.push('Ctrl');
                                if (e.shiftKey) modifiers.push('Shift');
                                if (e.altKey) modifiers.push('Alt');
                                if (e.metaKey) modifiers.push('Command');

                                const key = e.key.toUpperCase();
                                if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;

                                const newHotkey = [...modifiers, key].join('+');
                                props.updateRichPasteHotkey(newHotkey);
                            }}
                        >
                            {props.isRecordingRich ? (
                                <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                            ) : (
                                renderHotkeyCaps(props.richPasteHotkey)
                            )}
                        </div>
                    </div>
                    <div className="setting-item">
                        <div className="item-label-group">
                            <span className="item-label">{props.t('plain_paste_hotkey_label')}</span>
                            <span className="hint">{props.isRecordingPlain ? props.t('hotkey_recording_esc') : props.t('hotkey_click_hint')}</span>
                        </div>
                        <div
                            className={`key-group ${props.isRecordingPlain ? 'recording' : ''}`}
                            onClick={(e) => { beginRecording("plain"); e.currentTarget.focus(); }}
                            tabIndex={0}
                            onKeyDown={(e) => {
                                if (!props.isRecordingPlain) return;
                                e.preventDefault();
                                e.stopPropagation();

                                if (e.key === 'Escape') {
                                    props.setIsRecordingPlain(false);
                                    return;
                                }

                                if (e.key === 'Backspace' || e.key === 'Delete') {
                                    props.updatePlainPasteHotkey('');
                                    props.setIsRecordingPlain(false);
                                    return;
                                }

                                const modifiers = [];
                                if (e.ctrlKey) modifiers.push('Ctrl');
                                if (e.shiftKey) modifiers.push('Shift');
                                if (e.altKey) modifiers.push('Alt');
                                if (e.metaKey) modifiers.push('Command');

                                const key = e.key.toUpperCase();
                                if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;

                                const newHotkey = [...modifiers, key].join('+');
                                props.updatePlainPasteHotkey(newHotkey);
                            }}
                        >
                            {props.isRecordingPlain ? (
                                <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                            ) : (
                                renderHotkeyCaps(props.plainPasteHotkey)
                            )}
                        </div>
                    </div>
                    <div className="setting-item">
                        <div className="item-label-group">
                            <span className="item-label">{props.t('search_hotkey_label')}</span>
                            <span className="hint">{props.isRecordingSearch ? props.t('hotkey_recording_esc') : props.t('hotkey_click_hint')}</span>
                        </div>
                        <div
                            className={`key-group ${props.isRecordingSearch ? 'recording' : ''}`}
                            onClick={(e) => { beginRecording("search"); e.currentTarget.focus(); }}
                            tabIndex={0}
                            onKeyDown={(e) => {
                                if (!props.isRecordingSearch) return;
                                e.preventDefault();
                                e.stopPropagation();

                                if (e.key === 'Escape') {
                                    props.setIsRecordingSearch(false);
                                    return;
                                }

                                if (e.key === 'Backspace' || e.key === 'Delete') {
                                    props.updateSearchHotkey('');
                                    props.setIsRecordingSearch(false);
                                    return;
                                }

                                const modifiers = [];
                                if (e.ctrlKey) modifiers.push('Ctrl');
                                if (e.shiftKey) modifiers.push('Shift');
                                if (e.altKey) modifiers.push('Alt');
                                if (e.metaKey) modifiers.push('Command');

                                const key = e.key.toUpperCase();
                                if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;

                                const newHotkey = [...modifiers, key].join('+');
                                props.updateSearchHotkey(newHotkey);
                            }}
                        >
                            {props.isRecordingSearch ? (
                                <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                            ) : (
                                renderHotkeyCaps(props.searchHotkey)
                            )}
                        </div>
                    </div>
                    {props.relayAvailable && <div className="setting-item">
                        <div className="item-label-group">
                            <span className="item-label">{props.t('relay_send_hotkey_label')}</span>
                            <span className="hint">{props.isRecordingRelaySend ? props.t('hotkey_recording_esc') : props.t('relay_send_hotkey_hint')}</span>
                        </div>
                        <div
                            className={`key-group ${props.isRecordingRelaySend ? 'recording' : ''}`}
                            onClick={(e) => { beginRelayRecording("send"); e.currentTarget.focus(); }}
                            tabIndex={0}
                            onKeyDown={(e) => {
                                if (!props.isRecordingRelaySend) return;
                                e.preventDefault();
                                e.stopPropagation();
                                if (e.key === 'Escape') {
                                    props.setIsRecordingRelaySend(false);
                                    return;
                                }
                                if (e.key === 'Backspace' || e.key === 'Delete') {
                                    props.updateRelaySendHotkey('');
                                    return;
                                }
                                const modifiers = [];
                                if (e.ctrlKey) modifiers.push('Ctrl');
                                if (e.shiftKey) modifiers.push('Shift');
                                if (e.altKey) modifiers.push('Alt');
                                if (e.metaKey) modifiers.push('Command');
                                const key = e.key.toUpperCase();
                                if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;
                                props.updateRelaySendHotkey([...modifiers, key].join('+'));
                            }}
                        >
                            {props.isRecordingRelaySend ? (
                                <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                            ) : renderHotkeyCaps(props.relaySendHotkey)}
                        </div>
                    </div>}
                    {props.relayAvailable && <div className="setting-item">
                        <div className="item-label-group">
                            <span className="item-label">{props.t('relay_fetch_hotkey_label')}</span>
                            <span className="hint">{props.isRecordingRelayFetch ? props.t('hotkey_recording_esc') : props.t('relay_fetch_hotkey_hint')}</span>
                        </div>
                        <div
                            className={`key-group ${props.isRecordingRelayFetch ? 'recording' : ''}`}
                            onClick={(e) => { beginRelayRecording("fetch"); e.currentTarget.focus(); }}
                            tabIndex={0}
                            onKeyDown={(e) => {
                                if (!props.isRecordingRelayFetch) return;
                                e.preventDefault();
                                e.stopPropagation();
                                if (e.key === 'Escape') {
                                    props.setIsRecordingRelayFetch(false);
                                    return;
                                }
                                if (e.key === 'Backspace' || e.key === 'Delete') {
                                    props.updateRelayFetchHotkey('');
                                    return;
                                }
                                const modifiers = [];
                                if (e.ctrlKey) modifiers.push('Ctrl');
                                if (e.shiftKey) modifiers.push('Shift');
                                if (e.altKey) modifiers.push('Alt');
                                if (e.metaKey) modifiers.push('Command');
                                const key = e.key.toUpperCase();
                                if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;
                                props.updateRelayFetchHotkey([...modifiers, key].join('+'));
                            }}
                        >
                            {props.isRecordingRelayFetch ? (
                                <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                            ) : renderHotkeyCaps(props.relayFetchHotkey)}
                        </div>
                    </div>}
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('quick_paste_modifier')}
                            hint={props.t('quick_paste_modifier_hint')}
                            hintKey="quick_paste_modifier"
                        />
                        <select
                            value={props.quickPasteModifier}
                            onChange={(e) => {
                                const value = e.target.value as QuickPasteModifier;
                                props.setQuickPasteModifier(value);
                                invoke("set_quick_paste_modifier", { modifier: value }).catch(console.error);
                            }}
                            style={{
                                padding: '4px 8px',
                                borderRadius: '4px',
                                border: '1px solid var(--border-color)',
                                background: 'var(--input-bg)',
                                color: 'var(--text-color)',
                                fontSize: '14px',
                                minWidth: '140px'
                            }}
                        >
                            {quickPasteOptions.map((option) => (
                                <option key={option.value} value={option.value}>
                                    {option.label}
                                </option>
                            ))}
                        </select>
                    </div>
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('delete_after_paste')}
                            hint={props.t('delete_after_paste_hint')}
                            hintKey="delete_after_paste"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.deleteAfterPaste}
                                onChange={(e) => {
                                    const val = e.target.checked;
                                    props.setDeleteAfterPaste(val);
                                    props.saveAppSetting('delete_after_paste', String(val));
                                }}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('move_to_top_after_paste')}
                            hint={props.t('move_to_top_after_paste_hint')}
                            hintKey="move_to_top_after_paste"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.moveToTopAfterPaste}
                                onChange={(e) => {
                                    const val = e.target.checked;
                                    props.setMoveToTopAfterPaste(val);
                                    props.saveAppSetting('move_to_top_after_paste', String(val));
                                }}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>
                    {/* macOS cleanup: Removed Paste Method selection */}
                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('sequential_paste_mode')}
                            hint={props.t('sequential_paste_hint')}
                            hintKey="sequential_paste_mode"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.sequentialMode}
                                onChange={async (e) => {
                                    const input = e.currentTarget;
                                    const val = input.checked;
                                    try {
                                        await invoke('set_sequential_mode', { enabled: val });
                                        props.setSequentialModeState(val);
                                    } catch (err) {
                                        input.checked = props.sequentialMode;
                                        props.pushToast(
                                            `${props.t('hotkey_register_failed')}${err?.toString() || ''}`,
                                            5000
                                        );
                                    }
                                }}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>

                    <div className="setting-item">
                            <div className="item-label-group">
                                <span className="item-label">{props.t('sequential_paste_hotkey_label')}</span>
                                <span className="hint">{props.isRecordingSequential ? props.t('hotkey_recording_esc') : props.t('hotkey_click_hint')}</span>
                            </div>
                            <div
                                className={`key-group ${props.isRecordingSequential ? 'recording' : ''}`}
                                onClick={(e) => { beginRecording("sequential"); e.currentTarget.focus(); }}
                                tabIndex={0}
                                onKeyDown={(e) => {
                                    if (!props.isRecordingSequential) return;
                                    e.preventDefault();
                                    e.stopPropagation();

                                    if (e.key === 'Escape') {
                                        props.setIsRecordingSequential(false);
                                        return;
                                    }

                                    if (e.key === 'Backspace' || e.key === 'Delete') {
                                        props.updateSequentialHotkey('');
                                        props.setIsRecordingSequential(false);
                                        return;
                                    }

                                    const modifiers = [];
                                    if (e.ctrlKey) modifiers.push('Ctrl');
                                    if (e.shiftKey) modifiers.push('Shift');
                                    if (e.altKey) modifiers.push('Alt');
                                    if (e.metaKey) modifiers.push('Command');

                                    const key = e.key.toUpperCase();
                                    if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;

                                    const newHotkey = [...modifiers, key].join('+');
                                    props.updateSequentialHotkey(newHotkey);
                                }}
                            >
                                {props.isRecordingSequential ? (
                                    <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                                ) : (
                                    renderHotkeyCaps(props.sequentialHotkey)
                                )}
                            </div>
                        </div>

                    <div className="setting-item">
                        <props.LabelWithHint
                            label={props.t('privacy_protection')}
                            hint={props.t('privacy_protection_hint')}
                            hintKey="privacy_protection"
                        />
                        <label className="switch">
                            <input
                                className="cb"
                                type="checkbox"
                                checked={props.privacyProtection}
                                onChange={(e) => {
                                    const val = e.target.checked;
                                    props.setPrivacyProtection(val);
                                    invoke('set_privacy_protection', { enabled: val }).catch(console.error);
                                }}
                            />
                            <div className="toggle"><div className="left" /><div className="right" /></div>
                        </label>
                    </div>

                    <div className="setting-item" style={{ flexDirection: 'column', alignItems: 'flex-start', gap: '6px' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                            <button
                                type="button"
                                className="btn-icon"
                                onClick={() => props.setPrivacyKindsOpen(!props.privacyKindsOpen)}
                                style={{ width: '24px', height: '24px' }}
                            >
                                {props.privacyKindsOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                            </button>
                            <props.LabelWithHint
                                label={props.t('privacy_protection_kinds')}
                                hint={props.t('privacy_protection_kinds_hint')}
                                hintKey="privacy_protection_kinds"
                            />
                        </div>
                        {props.privacyKindsOpen && (
                            <div style={{ display: 'flex', flexWrap: 'wrap', gap: '8px', marginLeft: '30px' }}>
                                {[
                                    { id: 'url', label: props.t('privacy_kind_url') || '链接 / URL' },
                                    { id: 'phone', label: props.t('privacy_kind_phone') },
                                    { id: 'idcard', label: props.t('privacy_kind_idcard') },
                                    { id: 'email', label: props.t('privacy_kind_email') },
                                    { id: 'secret', label: props.t('privacy_kind_secret') },
                                    { id: 'password', label: props.t('privacy_kind_password') || "Strong Password" },
                                ].map(opt => {
                                    const checked = props.privacyProtectionKinds.includes(opt.id);
                                    return (
                                        <label key={opt.id} style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                                            <input
                                                className="cb"
                                                type="checkbox"
                                                checked={checked}
                                                onChange={(e) => {
                                                    const next = e.target.checked
                                                        ? [...props.privacyProtectionKinds, opt.id]
                                                        : props.privacyProtectionKinds.filter(t => t !== opt.id);
                                                    props.setPrivacyProtectionKinds(next);
                                                    invoke('set_privacy_protection_kinds', { kinds: next }).catch(console.error);
                                                }}
                                            />
                                            <span style={{ fontSize: '12px', color: 'var(--text-primary)' }}>{opt.label}</span>
                                        </label>
                                    );
                                })}
                            </div>
                        )}
                    </div>

                    <div className="setting-item" style={{ flexDirection: 'column', alignItems: 'flex-start', gap: '6px' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                            <button
                                type="button"
                                className="btn-icon"
                                onClick={() => props.setPrivacyRulesOpen(!props.privacyRulesOpen)}
                                style={{ width: '24px', height: '24px' }}
                            >
                                {props.privacyRulesOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                            </button>
                            <props.LabelWithHint
                                label={props.t('privacy_protection_custom_rules')}
                                hint={props.t('privacy_protection_custom_rules_hint')}
                                hintKey="privacy_protection_custom_rules"
                            />
                        </div>
                        {props.privacyRulesOpen && (
                            <textarea
                                className="search-input"
                                style={{ width: 'calc(100% - 30px)', maxWidth: '100%', minHeight: '80px', padding: '8px', borderRadius: '0', marginLeft: '30px', boxSizing: 'border-box' }}
                                placeholder={props.t('privacy_protection_custom_rules_placeholder')}
                                value={props.privacyProtectionCustomRules}
                                onFocus={() => invoke("focus_clipboard_window").catch(console.error)}
                                onChange={(e) => {
                                    const val = e.target.value;
                                    props.setPrivacyProtectionCustomRules(val);
                                    invoke('set_privacy_protection_custom_rules', { rules: val }).catch(console.error);
                                }}
                            />
                        )}
                    </div>

                    <div className="setting-item" style={{ flexDirection: 'column', alignItems: 'flex-start', gap: '6px' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                            <button
                                type="button"
                                className="btn-icon"
                                onClick={() => setMaskSettingsOpen(!maskSettingsOpen)}
                                style={{ width: '24px', height: '24px' }}
                            >
                                {maskSettingsOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                            </button>
                            <span className="item-label">{props.t('sensitive_mask_settings')}</span>
                        </div>
                        {maskSettingsOpen && (
                            <div style={{ width: 'calc(100% - 30px)', marginLeft: '30px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                                <div className="setting-item" style={{ padding: 0, borderBottom: 'none' }}>
                                    <span className="item-label">{props.t('sensitive_mask_prefix_visible')}</span>
                                    <input
                                        type="number"
                                        className="search-input"
                                        style={{ width: '60px', padding: '4px 8px', textAlign: 'center' }}
                                        min={0}
                                        max={20}
                                        value={props.sensitiveMaskPrefixVisible}
                                        onChange={(e) => {
                                            const val = Math.min(20, Math.max(0, parseInt(e.target.value) || 0));
                                            props.setSensitiveMaskPrefixVisible(val);
                                            invoke('save_setting', { key: 'app.sensitive_mask_prefix_visible', value: val.toString() }).catch(console.error);
                                        }}
                                    />
                                </div>
                                <div className="setting-item" style={{ padding: 0, borderBottom: 'none' }}>
                                    <span className="item-label">{props.t('sensitive_mask_suffix_visible')}</span>
                                    <input
                                        type="number"
                                        className="search-input"
                                        style={{ width: '60px', padding: '4px 8px', textAlign: 'center' }}
                                        min={0}
                                        max={20}
                                        value={props.sensitiveMaskSuffixVisible}
                                        onChange={(e) => {
                                            const val = Math.min(20, Math.max(0, parseInt(e.target.value) || 0));
                                            props.setSensitiveMaskSuffixVisible(val);
                                            invoke('save_setting', { key: 'app.sensitive_mask_suffix_visible', value: val.toString() }).catch(console.error);
                                        }}
                                    />
                                </div>
                                <div className="setting-item" style={{ padding: 0, borderBottom: 'none' }}>
                                    <props.LabelWithHint
                                        label={props.t('sensitive_mask_email_domain')}
                                        hint={props.t('sensitive_mask_email_domain_hint')}
                                        hintKey="sensitive_mask_email_domain"
                                    />
                                    <label className="switch">
                                        <input
                                            type="checkbox"
                                            checked={props.sensitiveMaskEmailDomain}
                                            onChange={(e) => {
                                                props.setSensitiveMaskEmailDomain(e.target.checked);
                                                invoke('save_setting', { key: 'app.sensitive_mask_email_domain', value: e.target.checked.toString() }).catch(console.error);
                                            }}
                                        />
                                        <span className="slider" />
                                    </label>
                                </div>
                            </div>
                        )}
                    </div>

                    {(!isWindows || !props.registryWinVEnabled) && (
                        <div className="setting-item no-border">
                        <div className="item-label-group">
                            <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                                <span className="item-label">{props.t('global_hotkey')}</span>
                            </div>
                            <span className="hint">{props.isRecording ? props.t('hotkey_recording_esc') : props.t('hotkey_click_hint')}</span>
                        </div>

                        <div
                            className={`key-group ${props.isRecording ? 'recording' : ''}`}
                            onClick={(e) => { beginRecording("main"); e.currentTarget.focus(); }}
                            tabIndex={0}
                            onKeyDown={(e) => {
                                if (!props.isRecording) return;
                                e.preventDefault();
                                e.stopPropagation();

                                if (e.key === 'Escape') {
                                    props.setIsRecording(false);
                                    return;
                                }

                                if (e.key === 'Backspace' || e.key === 'Delete') {
                                    props.updateHotkey('');
                                    props.setIsRecording(false);
                                    return;
                                }

                                const modifiers = [];
                                if (e.ctrlKey) modifiers.push('Ctrl');
                                if (e.shiftKey) modifiers.push('Shift');
                                if (e.altKey) modifiers.push('Alt');
                                if (e.metaKey) modifiers.push('Command');

                                const key = e.key.toUpperCase();
                                if (['CONTROL', 'SHIFT', 'ALT', 'META'].includes(key)) return;

                                const newHotkey = [...modifiers, key].join('+');
                                props.updateHotkey(newHotkey);
                            }}
                        >
                            {props.isRecording ? (
                                <div className="key-cap" style={{ width: '8em' }}>{props.t('waiting_for_input')}</div>
                            ) : (
                                renderHotkeyCaps(props.hotkey)
                            )}
                        </div>
                        </div>
                    )}

                    {isWindows && (
                        <div className="setting-item">
                            <props.LabelWithHint
                                label={props.t('use_win_v_shortcut')}
                                hint={props.t('use_win_v_shortcut_hint')}
                                hintKey="use_win_v_shortcut"
                            />
                            <label className="switch">
                                <input
                                    className="cb"
                                    type="checkbox"
                                    checked={props.registryWinVEnabled}
                                    disabled={winVUpdating}
                                    onChange={(event) => void updateWinVShortcut(event.target.checked)}
                                />
                                <div className="toggle"><div className="left" /><div className="right" /></div>
                            </label>
                        </div>
                    )}
                </div>
            )}
        </div>
    );
};

export default ClipboardSettingsGroup;
