import { useEffect } from "react";
import { DEFAULT_COLLAPSED_GROUPS } from "../../features/settings/defaultCollapsedGroups";

interface UseSettingsPanelResetOptions {
  showSettings: boolean;
  setCollapsedGroups: (val: Record<string, boolean>) => void;
  setSettingsSubpage: (val: "home" | "advanced") => void;
}

export const useSettingsPanelReset = ({
  showSettings,
  setCollapsedGroups,
  setSettingsSubpage
}: UseSettingsPanelResetOptions) => {
  useEffect(() => {
    if (showSettings) {
      setSettingsSubpage("home");
      setCollapsedGroups({ ...DEFAULT_COLLAPSED_GROUPS });
    }
  }, [showSettings, setCollapsedGroups, setSettingsSubpage]);
};
