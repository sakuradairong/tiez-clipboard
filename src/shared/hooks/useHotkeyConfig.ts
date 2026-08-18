import { useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type HotkeyMode = "main" | "sequential" | "rich" | "plain" | "search" | "relaySend" | "relayFetch";

const canonicalHotkey = (value: string) => {
  const aliases: Record<string, string> = {
    command: "super",
    cmd: "super",
    win: "super",
    meta: "super",
    option: "alt",
    control: "ctrl"
  };
  const rank: Record<string, number> = { ctrl: 0, shift: 1, alt: 2, super: 3 };
  return value
    .split("+")
    .map((part) => aliases[part.trim().toLowerCase()] || part.trim().toLowerCase())
    .filter(Boolean)
    .sort((left, right) => (rank[left] ?? 10) - (rank[right] ?? 10) || left.localeCompare(right))
    .join("+");
};

interface UseHotkeyConfigOptions {
  hotkey: string;
  setHotkey: (val: string) => void;
  sequentialHotkey: string;
  setSequentialHotkey: (val: string) => void;
  richPasteHotkey: string;
  setRichPasteHotkey: (val: string) => void;
  plainPasteHotkey: string;
  setPlainPasteHotkey: (val: string) => void;
  searchHotkey: string;
  setSearchHotkey: (val: string) => void;
  relaySendHotkey: string;
  setRelaySendHotkey: (val: string) => void;
  relayFetchHotkey: string;
  setRelayFetchHotkey: (val: string) => void;
  sequentialMode: boolean;
  isRecording: boolean;
  setIsRecording: (val: boolean) => void;
  isRecordingSequential: boolean;
  setIsRecordingSequential: (val: boolean) => void;
  isRecordingRich: boolean;
  setIsRecordingRich: (val: boolean) => void;
  isRecordingPlain: boolean;
  setIsRecordingPlain: (val: boolean) => void;
  isRecordingSearch: boolean;
  setIsRecordingSearch: (val: boolean) => void;
  isRecordingRelaySend: boolean;
  setIsRecordingRelaySend: (val: boolean) => void;
  isRecordingRelayFetch: boolean;
  setIsRecordingRelayFetch: (val: boolean) => void;
  saveAppSetting: (type: string, value: string) => void;
  t: (key: string) => string;
  pushToast: (msg: string, duration?: number) => number;
}

const isUnrecordableHotkeyPayload = (payload: string) =>
  payload === "Backspace" || payload === "Delete";

export const useHotkeyConfig = ({
  hotkey,
  setHotkey,
  sequentialHotkey,
  setSequentialHotkey,
  richPasteHotkey,
  setRichPasteHotkey,
  plainPasteHotkey,
  setPlainPasteHotkey,
  searchHotkey,
  setSearchHotkey,
  relaySendHotkey,
  setRelaySendHotkey,
  relayFetchHotkey,
  setRelayFetchHotkey,
  sequentialMode,
  isRecording,
  setIsRecording,
  isRecordingSequential,
  setIsRecordingSequential,
  isRecordingRich,
  setIsRecordingRich,
  isRecordingPlain,
  setIsRecordingPlain,
  isRecordingSearch,
  setIsRecordingSearch,
  isRecordingRelaySend,
  setIsRecordingRelaySend,
  isRecordingRelayFetch,
  setIsRecordingRelayFetch,
  t,
  pushToast
}: UseHotkeyConfigOptions) => {
  const checkHotkeyConflict = useCallback(
    (newHotkey: string, mode: HotkeyMode): boolean => {
      if (!newHotkey) return false;

      const candidate = canonicalHotkey(newHotkey);
      const conflicts = [];
      if (mode !== "main" && candidate === canonicalHotkey(hotkey)) conflicts.push(t("global_hotkey"));
      if (mode !== "sequential" && sequentialMode && candidate === canonicalHotkey(sequentialHotkey)) {
        conflicts.push(t("sequential_paste_hotkey_label"));
      }
      if (mode !== "rich" && candidate === canonicalHotkey(richPasteHotkey)) {
        conflicts.push(t("rich_paste_hotkey_label"));
      }
      if (mode !== "plain" && candidate === canonicalHotkey(plainPasteHotkey)) {
        conflicts.push(t("plain_paste_hotkey_label"));
      }
      if (mode !== "search" && candidate === canonicalHotkey(searchHotkey)) {
        conflicts.push(t("search_hotkey_label"));
      }
      if (mode !== "relaySend" && candidate === canonicalHotkey(relaySendHotkey)) {
        conflicts.push(t("relay_send_hotkey_label"));
      }
      if (mode !== "relayFetch" && candidate === canonicalHotkey(relayFetchHotkey)) {
        conflicts.push(t("relay_fetch_hotkey_label"));
      }

      if (conflicts.length > 0) {
        const msg = t("hotkey_conflict_toast").replace("{name}", conflicts[0]);
        pushToast(msg, 5000);
        return true;
      }
      return false;
    },
    [hotkey, sequentialMode, sequentialHotkey, richPasteHotkey, plainPasteHotkey, searchHotkey, relaySendHotkey, relayFetchHotkey, t, pushToast]
  );

  const updateHotkey = useCallback(
    async (newHotkey: string) => {
      const hasConflict = checkHotkeyConflict(newHotkey, "main");
      if (hasConflict) {
        setIsRecording(false);
        return false;
      }

      if (newHotkey) {
        try {
          const available = await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
          if (!available) throw new Error("快捷键不可用");
        } catch (err) {
          const errorMsg = `❌ ${newHotkey}: ${err || "快捷键被占用"}`;
          pushToast(errorMsg, 5000);
          setIsRecording(false);
          return false;
        }
      }

      try {
        await invoke("register_hotkey", { hotkey: newHotkey });
        setHotkey(newHotkey);
        return true;
      } catch (err) {
        const errorMsg = t("hotkey_register_failed") + (err?.toString() || "");
        pushToast(errorMsg, 5000);
        return false;
      } finally {
        setIsRecording(false);
      }
    },
    [checkHotkeyConflict, pushToast, setHotkey, setIsRecording, t]
  );

  const updateSequentialHotkey = useCallback(
    async (newHotkey: string) => {
      const hasConflict = checkHotkeyConflict(newHotkey, "sequential");
      if (hasConflict) {
        setIsRecordingSequential(false);
        return;
      }

      if (newHotkey) {
        try {
          await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
        } catch (err) {
          const errorMsg = `❌ ${newHotkey}: ${err || "快捷键被占用"}`;
          pushToast(errorMsg, 5000);
          setIsRecordingSequential(false);
          return;
        }
      }

      try {
        await invoke("set_sequential_hotkey", { hotkey: newHotkey });
        setSequentialHotkey(newHotkey);
      } catch (err) {
        pushToast(`${t("hotkey_register_failed")}${err?.toString() || ""}`, 5000);
      } finally {
        setIsRecordingSequential(false);
      }
    },
    [
      checkHotkeyConflict,
      pushToast,
      setSequentialHotkey,
      setIsRecordingSequential,
      t
    ]
  );

  const updateRichPasteHotkey = useCallback(
    async (newHotkey: string) => {
      const hasConflict = checkHotkeyConflict(newHotkey, "rich");
      if (hasConflict) {
        setIsRecordingRich(false);
        return;
      }

      if (newHotkey) {
        try {
          await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
        } catch (err) {
          const errorMsg = `❌ ${newHotkey}: ${err || "快捷键被占用"}`;
          pushToast(errorMsg, 5000);
          setIsRecordingRich(false);
          return;
        }
      }

      try {
        await invoke("set_rich_paste_hotkey", { hotkey: newHotkey });
        setRichPasteHotkey(newHotkey);
      } catch (err) {
        pushToast(`${t("hotkey_register_failed")}${err?.toString() || ""}`, 5000);
      } finally {
        setIsRecordingRich(false);
      }
    },
    [
      checkHotkeyConflict,
      pushToast,
      setRichPasteHotkey,
      setIsRecordingRich,
      t
    ]
  );

  const updatePlainPasteHotkey = useCallback(
    async (newHotkey: string) => {
      const hasConflict = checkHotkeyConflict(newHotkey, "plain");
      if (hasConflict) {
        setIsRecordingPlain(false);
        return;
      }

      if (newHotkey) {
        try {
          await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
        } catch (err) {
          const errorMsg = `❌ ${newHotkey}: ${err || "快捷键被占用"}`;
          pushToast(errorMsg, 5000);
          setIsRecordingPlain(false);
          return;
        }
      }

      try {
        await invoke("set_plain_paste_hotkey", { hotkey: newHotkey });
        setPlainPasteHotkey(newHotkey);
      } catch (err) {
        const errorMsg = `${t("hotkey_register_failed")}${err?.toString() || ""}`;
        pushToast(errorMsg, 5000);
      } finally {
        setIsRecordingPlain(false);
      }
    },
    [checkHotkeyConflict, pushToast, setPlainPasteHotkey, setIsRecordingPlain, t]
  );

  const updateSearchHotkey = useCallback(
    async (newHotkey: string) => {
      const hasConflict = checkHotkeyConflict(newHotkey, "search");
      if (hasConflict) {
        setIsRecordingSearch(false);
        return;
      }

      if (newHotkey) {
        try {
          await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
        } catch (err) {
          const errorMsg = `❌ ${newHotkey}: ${err || "快捷键被占用"}`;
          pushToast(errorMsg, 5000);
          setIsRecordingSearch(false);
          return;
        }
      }

      try {
        await invoke("set_search_hotkey", { hotkey: newHotkey });
        setSearchHotkey(newHotkey);
      } catch (err) {
        pushToast(`${t("hotkey_register_failed")}${err?.toString() || ""}`, 5000);
      } finally {
        setIsRecordingSearch(false);
      }
    },
    [
      checkHotkeyConflict,
      pushToast,
      setSearchHotkey,
      setIsRecordingSearch,
      t
    ]
  );

  const updateRelayHotkey = useCallback(
    async (
      newHotkey: string,
      mode: "relaySend" | "relayFetch",
      command: "set_relay_send_hotkey" | "set_relay_fetch_hotkey",
      setHotkeyValue: (value: string) => void,
      setRecording: (value: boolean) => void
    ) => {
      if (checkHotkeyConflict(newHotkey, mode)) {
        setRecording(false);
        return;
      }
      if (newHotkey) {
        try {
          await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
        } catch (err) {
          pushToast(`❌ ${newHotkey}: ${err || "快捷键被占用"}`, 5000);
          setRecording(false);
          return;
        }
      }
      try {
        await invoke(command, { hotkey: newHotkey });
        setHotkeyValue(newHotkey);
      } catch (err) {
        pushToast(`${t("hotkey_register_failed")}${err?.toString() || ""}`, 5000);
      } finally {
        setRecording(false);
      }
    },
    [checkHotkeyConflict, pushToast, t]
  );

  const updateRelaySendHotkey = useCallback(
    (newHotkey: string) => updateRelayHotkey(
      newHotkey,
      "relaySend",
      "set_relay_send_hotkey",
      setRelaySendHotkey,
      setIsRecordingRelaySend
    ),
    [setIsRecordingRelaySend, setRelaySendHotkey, updateRelayHotkey]
  );

  const updateRelayFetchHotkey = useCallback(
    (newHotkey: string) => updateRelayHotkey(
      newHotkey,
      "relayFetch",
      "set_relay_fetch_hotkey",
      setRelayFetchHotkey,
      setIsRecordingRelayFetch
    ),
    [setIsRecordingRelayFetch, setRelayFetchHotkey, updateRelayHotkey]
  );

  useEffect(() => {
    invoke("set_recording_mode", {
      enabled: isRecording || isRecordingSequential || isRecordingRich
        || isRecordingPlain || isRecordingSearch || isRecordingRelaySend || isRecordingRelayFetch
    }).catch(console.error);

    if (isRecording || isRecordingSequential || isRecordingRich || isRecordingPlain || isRecordingSearch || isRecordingRelaySend || isRecordingRelayFetch) {
      const unlisten = listen<string>("hotkey-recorded", (event) => {
        if (isUnrecordableHotkeyPayload(event.payload)) {
          return;
        }
        if (isRecording) updateHotkey(event.payload);
        if (isRecordingSequential) updateSequentialHotkey(event.payload);
        if (isRecordingRich) updateRichPasteHotkey(event.payload);
        if (isRecordingPlain) updatePlainPasteHotkey(event.payload);
        if (isRecordingSearch) updateSearchHotkey(event.payload);
        if (isRecordingRelaySend) updateRelaySendHotkey(event.payload);
        if (isRecordingRelayFetch) updateRelayFetchHotkey(event.payload);
      });

      const unlistenClear = listen("hotkey-cleared", () => {
        if (isRecording) updateHotkey("");
        if (isRecordingSequential) updateSequentialHotkey("");
        if (isRecordingRich) updateRichPasteHotkey("");
        if (isRecordingPlain) updatePlainPasteHotkey("");
        if (isRecordingSearch) updateSearchHotkey("");
        if (isRecordingRelaySend) updateRelaySendHotkey("");
        if (isRecordingRelayFetch) updateRelayFetchHotkey("");
      });

      const unlistenCancel = listen("recording-cancelled", () => {
        setIsRecording(false);
        setIsRecordingSequential(false);
        setIsRecordingRich(false);
        setIsRecordingPlain(false);
        setIsRecordingSearch(false);
        setIsRecordingRelaySend(false);
        setIsRecordingRelayFetch(false);
      });

      return () => {
        unlisten.then((f) => f());
        unlistenClear.then((f) => f());
        unlistenCancel.then((f) => f());
      };
    }
  }, [
    isRecording,
    isRecordingSequential,
    isRecordingRich,
    isRecordingPlain,
    isRecordingSearch,
    isRecordingRelaySend,
    isRecordingRelayFetch,
    setIsRecording,
    setIsRecordingSequential,
    setIsRecordingRich,
    setIsRecordingPlain,
    setIsRecordingSearch,
    setIsRecordingRelaySend,
    setIsRecordingRelayFetch,
    updateHotkey,
    updateSequentialHotkey,
    updateRichPasteHotkey,
    updatePlainPasteHotkey,
    updateSearchHotkey,
    updateRelaySendHotkey,
    updateRelayFetchHotkey
  ]);

  return {
    checkHotkeyConflict,
    updateHotkey,
    updateSequentialHotkey,
    updateRichPasteHotkey,
    updatePlainPasteHotkey,
    updateSearchHotkey,
    updateRelaySendHotkey,
    updateRelayFetchHotkey
  };
};
