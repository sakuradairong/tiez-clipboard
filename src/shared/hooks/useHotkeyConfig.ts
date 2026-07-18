import { useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type HotkeyMode = "main" | "sequential" | "rich" | "plain" | "search";

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
  saveAppSetting: (type: string, value: string) => void;
  t: (key: string) => string;
  pushToast: (msg: string, duration?: number) => number;
}

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
  saveAppSetting,
  t,
  pushToast
}: UseHotkeyConfigOptions) => {
  const checkHotkeyConflict = useCallback(
    (newHotkey: string, mode: HotkeyMode): boolean => {
      if (!newHotkey) return false;

      const conflicts = [];
      if (mode !== "main" && newHotkey === hotkey) conflicts.push(t("global_hotkey"));
      if (mode !== "sequential" && sequentialMode && newHotkey === sequentialHotkey) {
        conflicts.push(t("sequential_paste_hotkey_label"));
      }
      if (mode !== "rich" && newHotkey === richPasteHotkey) {
        conflicts.push(t("rich_paste_hotkey_label"));
      }
      if (mode !== "plain" && newHotkey === plainPasteHotkey) {
        conflicts.push(t("plain_paste_hotkey_label"));
      }
      if (mode !== "search" && newHotkey === searchHotkey) {
        conflicts.push(t("search_hotkey_label"));
      }

      if (conflicts.length > 0) {
        const msg = t("hotkey_conflict_toast").replace("{name}", conflicts[0]);
        pushToast(msg, 5000);
        return true;
      }
      return false;
    },
    [hotkey, sequentialMode, sequentialHotkey, richPasteHotkey, plainPasteHotkey, searchHotkey, t, pushToast]
  );

  const updateHotkey = useCallback(
    async (newHotkey: string) => {
      const hasConflict = checkHotkeyConflict(newHotkey, "main");
      if (hasConflict) {
        setIsRecording(false);
        return;
      }

      if (newHotkey) {
        try {
          await invoke<boolean>("test_hotkey_available", { hotkey: newHotkey });
        } catch (err) {
          const errorMsg = `❌ ${newHotkey}: ${err || "快捷键被占用"}`;
          pushToast(errorMsg, 5000);
          setIsRecording(false);
          return;
        }
      }

      setHotkey(newHotkey);
      saveAppSetting("hotkey", newHotkey);
      await invoke("register_hotkey", { hotkey: newHotkey }).catch((err) => {
        if (newHotkey) {
          const errorMsg = t("hotkey_register_failed") + (err?.toString() || "");
          pushToast(errorMsg, 3000);
        }
      });
      setIsRecording(false);
    },
    [checkHotkeyConflict, pushToast, saveAppSetting, setHotkey, setIsRecording, t]
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

      setSequentialHotkey(newHotkey);
      saveAppSetting("sequential_hotkey", newHotkey);
      await invoke("set_sequential_hotkey", { hotkey: newHotkey }).catch(console.error);
      setIsRecordingSequential(false);
    },
    [
      checkHotkeyConflict,
      pushToast,
      saveAppSetting,
      setSequentialHotkey,
      setIsRecordingSequential
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

      setRichPasteHotkey(newHotkey);
      saveAppSetting("rich_paste_hotkey", newHotkey);
      await invoke("set_rich_paste_hotkey", { hotkey: newHotkey }).catch(console.error);
      setIsRecordingRich(false);
    },
    [
      checkHotkeyConflict,
      pushToast,
      saveAppSetting,
      setRichPasteHotkey,
      setIsRecordingRich
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

      setPlainPasteHotkey(newHotkey);
      saveAppSetting("plain_paste_hotkey", newHotkey);
      await invoke("set_plain_paste_hotkey", { hotkey: newHotkey }).catch(console.error);
      setIsRecordingPlain(false);
    },
    [checkHotkeyConflict, pushToast, saveAppSetting, setPlainPasteHotkey, setIsRecordingPlain]
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

      setSearchHotkey(newHotkey);
      saveAppSetting("search_hotkey", newHotkey);
      await invoke("set_search_hotkey", { hotkey: newHotkey }).catch(console.error);
      setIsRecordingSearch(false);
    },
    [
      checkHotkeyConflict,
      pushToast,
      saveAppSetting,
      setSearchHotkey,
      setIsRecordingSearch
    ]
  );

  useEffect(() => {
    invoke("set_recording_mode", {
      enabled: isRecording || isRecordingSequential || isRecordingRich
        || isRecordingPlain || isRecordingSearch
    }).catch(console.error);

    if (isRecording || isRecordingSequential || isRecordingRich || isRecordingPlain || isRecordingSearch) {
      const unlisten = listen<string>("hotkey-recorded", (event) => {
        if (isRecording) updateHotkey(event.payload);
        if (isRecordingSequential) updateSequentialHotkey(event.payload);
        if (isRecordingRich) updateRichPasteHotkey(event.payload);
        if (isRecordingPlain) updatePlainPasteHotkey(event.payload);
        if (isRecordingSearch) updateSearchHotkey(event.payload);
      });

      const unlistenCancel = listen("recording-cancelled", () => {
        setIsRecording(false);
        setIsRecordingSequential(false);
        setIsRecordingRich(false);
        setIsRecordingPlain(false);
        setIsRecordingSearch(false);
      });

      return () => {
        unlisten.then((f) => f());
        unlistenCancel.then((f) => f());
      };
    }
  }, [
    isRecording,
    isRecordingSequential,
    isRecordingRich,
    isRecordingPlain,
    isRecordingSearch,
    setIsRecording,
    setIsRecordingSequential,
    setIsRecordingRich,
    setIsRecordingPlain,
    setIsRecordingSearch,
    updateHotkey,
    updateSequentialHotkey,
    updateRichPasteHotkey,
    updatePlainPasteHotkey,
    updateSearchHotkey
  ]);

  return {
    checkHotkeyConflict,
    updateHotkey,
    updateSequentialHotkey,
    updateRichPasteHotkey,
    updatePlainPasteHotkey,
    updateSearchHotkey
  };
};
