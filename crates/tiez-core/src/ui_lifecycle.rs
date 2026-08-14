//! Minimal main-window lifecycle policy for native TieZ shells.
//!
//! Platform code owns RegisterHotKey, HWND capture, and ShowWindow. This
//! module decides whether a hotkey, Escape, or deactivate should show, hide,
//! or leave the window alone, including the last-foreground capture contract
//! used before paste.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiLifecycleState {
    pub visible: bool,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiLifecycleEvent {
    ToggleHotkey,
    Escape,
    Deactivated,
    ShowRequested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiLifecycleCommand {
    Show { capture_foreground: bool },
    Hide,
    Noop,
}

pub fn plan_lifecycle(state: &UiLifecycleState, event: UiLifecycleEvent) -> UiLifecycleCommand {
    match event {
        UiLifecycleEvent::ToggleHotkey | UiLifecycleEvent::ShowRequested => {
            if state.visible && matches!(event, UiLifecycleEvent::ToggleHotkey) {
                UiLifecycleCommand::Hide
            } else {
                UiLifecycleCommand::Show {
                    capture_foreground: true,
                }
            }
        }
        UiLifecycleEvent::Escape => {
            if state.visible {
                UiLifecycleCommand::Hide
            } else {
                UiLifecycleCommand::Noop
            }
        }
        UiLifecycleEvent::Deactivated => {
            if state.visible && !state.pinned {
                UiLifecycleCommand::Hide
            } else {
                UiLifecycleCommand::Noop
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(pinned: bool) -> UiLifecycleState {
        UiLifecycleState {
            visible: true,
            pinned,
        }
    }

    fn hidden() -> UiLifecycleState {
        UiLifecycleState {
            visible: false,
            pinned: false,
        }
    }

    #[test]
    fn hotkey_toggles_visibility_and_captures_foreground_on_show() {
        assert_eq!(
            plan_lifecycle(&hidden(), UiLifecycleEvent::ToggleHotkey),
            UiLifecycleCommand::Show {
                capture_foreground: true
            }
        );
        assert_eq!(
            plan_lifecycle(&visible(false), UiLifecycleEvent::ToggleHotkey),
            UiLifecycleCommand::Hide
        );
    }

    #[test]
    fn escape_hides_a_visible_window() {
        assert_eq!(
            plan_lifecycle(&visible(true), UiLifecycleEvent::Escape),
            UiLifecycleCommand::Hide
        );
        assert_eq!(
            plan_lifecycle(&hidden(), UiLifecycleEvent::Escape),
            UiLifecycleCommand::Noop
        );
    }

    #[test]
    fn deactivate_hides_unless_the_window_is_pinned() {
        assert_eq!(
            plan_lifecycle(&visible(false), UiLifecycleEvent::Deactivated),
            UiLifecycleCommand::Hide
        );
        assert_eq!(
            plan_lifecycle(&visible(true), UiLifecycleEvent::Deactivated),
            UiLifecycleCommand::Noop
        );
        assert_eq!(
            plan_lifecycle(&hidden(), UiLifecycleEvent::Deactivated),
            UiLifecycleCommand::Noop
        );
    }
}
