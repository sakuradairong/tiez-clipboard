//! Pure Windows paste-target rules shared by legacy and native hosts.
pub(crate) fn matches_target_foreground(target: usize, foreground: usize) -> bool {
    target != 0 && target == foreground
}
pub(crate) fn is_system_focus_class(class_name: &str) -> bool {
    matches!(
        class_name,
        // Taskbar / tray / shell surfaces (these should NOT receive paste)
        "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "TrayNotifyWnd"
            | "NotifyIconOverflowWindow"
            | "ReBarWindow32"
            | "MSTaskSwWClass"
            // Note: Progman and WorkerW (desktop windows) are intentionally NOT filtered
            // because users may need to paste when renaming files on the desktop
            | "Button"
            // System UI overlays that should NOT receive paste
            | "ImmersiveLauncher"
            | "ShellExperienceHost"
            | "TaskSwitcherWnd"
            | "MultitaskingViewFrame" // Note: Windows.UI.Core.CoreWindow, SearchUI, Cortana, XamlExplorerHostIslandWindow,
                                      // and ApplicationFrameWindow are intentionally NOT filtered because users may need
                                      // to paste in Windows search box and other UWP app input fields
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_requires_exact_nonzero_target() {
        assert!(matches_target_foreground(42, 42));
        for (target, foreground) in [(0, 0), (42, 0), (0, 42), (42, 43)] {
            assert!(!matches_target_foreground(target, foreground));
        }
    }
    #[test]
    fn shell_overlays_are_excluded_but_desktop_rename_and_uwp_remain_allowed() {
        for name in [
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "TrayNotifyWnd",
            "NotifyIconOverflowWindow",
            "ReBarWindow32",
            "MSTaskSwWClass",
            "Button",
            "ImmersiveLauncher",
            "ShellExperienceHost",
            "TaskSwitcherWnd",
            "MultitaskingViewFrame",
        ] {
            assert!(is_system_focus_class(name), "{name}");
        }
        for name in [
            "Progman",
            "WorkerW",
            "Windows.UI.Core.CoreWindow",
            "ApplicationFrameWindow",
            "SearchUI",
            "XamlExplorerHostIslandWindow",
            "Cortana",
            "Notepad",
            "Chrome_WidgetWin_1",
        ] {
            assert!(!is_system_focus_class(name), "{name}");
        }
    }
}
