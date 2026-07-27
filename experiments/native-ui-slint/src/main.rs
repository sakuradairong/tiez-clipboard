use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, VecModel};
use std::{cell::RefCell, rc::Rc, time::Duration};

slint::slint! {
    import { Button, LineEdit, ListView, VerticalBox, HorizontalBox } from "std-widgets.slint";

    export struct ClipRow {
        id: int,
        kind: string,
        preview: string,
        source: string,
        timestamp: string,
        pinned: bool,
    }

    export component NativeClipboardWindow inherits Window {
        title: "TieZ native UI feasibility PoC";
        width: 420px;
        height: 680px;
        background: #121419;

        in property <[ClipRow]> rows;
        in-out property <int> selected-index: 0;
        in property <string> status-text: "";
        callback search-edited(string);
        callback move-selection(int);
        callback activate-selection();
        callback hide-temporarily();

        VerticalBox {
            padding: 12px;
            spacing: 8px;

            Text {
                text: "TieZ  ·  Native list PoC";
                color: #f7f8fa;
                font-size: 20px;
                font-weight: 700;
            }

            HorizontalBox {
                spacing: 6px;
                search-box := LineEdit {
                    placeholder-text: "Search clipboard history";
                    edited(value) => { root.search-edited(value); }
                    accepted(value) => { root.activate-selection(); }
                    key-pressed(event) => {
                        if (event.text == Key.DownArrow) {
                            root.move-selection(1);
                            return accept;
                        } else if (event.text == Key.UpArrow) {
                            root.move-selection(-1);
                            return accept;
                        }
                        return reject;
                    }
                }
                Button {
                    text: "Hide 2s";
                    clicked => { root.hide-temporarily(); }
                }
            }

            Text {
                text: root.status-text;
                color: #98a1b2;
                font-size: 12px;
            }

            ListView {
                for row[index] in root.rows: Rectangle {
                    height: 76px;
                    background: index == root.selected-index ? #263b5f :
                                touch.has-hover ? #20242d : #181b22;
                    border-radius: 8px;

                    touch := TouchArea {
                        clicked => {
                            root.selected-index = index;
                            root.activate-selection();
                        }
                    }

                    HorizontalBox {
                        padding: 9px;
                        spacing: 10px;

                        Rectangle {
                            width: 48px;
                            height: 48px;
                            border-radius: 7px;
                            background: row.kind == "image" ? rgb(101, 78, 163) :
                                        row.kind == "file" ? #3a6f55 : #33415c;
                            Text {
                                text: row.kind == "image" ? "IMG" :
                                      row.kind == "file" ? "FILE" : "TXT";
                                color: white;
                                font-size: 11px;
                                horizontal-alignment: center;
                                vertical-alignment: center;
                            }
                        }

                        VerticalBox {
                            spacing: 2px;
                            Text {
                                text: (row.pinned ? "★  " : "") + row.preview;
                                color: #f2f4f8;
                                font-size: 14px;
                                overflow: elide;
                            }
                            Text {
                                text: row.source + "  ·  " + row.timestamp;
                                color: #929bad;
                                font-size: 11px;
                                overflow: elide;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn make_rows(count: usize) -> Vec<ClipRow> {
    (0..count)
        .map(|index| {
            let kind = match index % 11 {
                0 => "image",
                1 => "file",
                _ => "text",
            };
            let preview = match kind {
                "image" => format!("Screenshot {index} · 1920 × 1080 (image placeholder)"),
                "file" => format!("C:\\Documents\\sample-{index}.pdf"),
                _ => format!("Clipboard item {index}: searchable text with preserved whitespace"),
            };
            ClipRow {
                id: index as i32,
                kind: kind.into(),
                preview: preview.into(),
                source: if index % 2 == 0 { "Code" } else { "Browser" }.into(),
                timestamp: format!("{:02}:{:02}", (index / 60) % 24, index % 60).into(),
                pinned: index < 4,
            }
        })
        .collect()
}

fn filtered_rows(rows: &[ClipRow], query: &str) -> Vec<ClipRow> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|row| {
            row.preview.to_lowercase().contains(&query)
                || row.source.to_lowercase().contains(&query)
                || row.kind.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

fn set_rows(ui: &NativeClipboardWindow, rows: Vec<ClipRow>, total: usize) {
    let visible = rows.len();
    ui.set_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_selected_index(0);
    ui.set_status_text(SharedString::from(format!(
        "Showing {visible} of {total} synthetic entries · virtualized ListView"
    )));
}

fn main() -> Result<(), slint::PlatformError> {
    let row_count = std::env::var("TIEZ_NATIVE_POC_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000);
    let all_rows = Rc::new(make_rows(row_count));
    let ui = NativeClipboardWindow::new()?;
    set_rows(&ui, all_rows.as_ref().clone(), all_rows.len());

    {
        let ui = ui.as_weak();
        let all_rows = all_rows.clone();
        ui.upgrade().unwrap().on_search_edited(move |query| {
            if let Some(ui) = ui.upgrade() {
                set_rows(
                    &ui,
                    filtered_rows(&all_rows, query.as_str()),
                    all_rows.len(),
                );
            }
        });
    }

    {
        let ui = ui.as_weak();
        ui.upgrade().unwrap().on_move_selection(move |delta| {
            if let Some(ui) = ui.upgrade() {
                let count = ui.get_rows().row_count() as i32;
                if count > 0 {
                    ui.set_selected_index((ui.get_selected_index() + delta).clamp(0, count - 1));
                }
            }
        });
    }

    {
        let ui = ui.as_weak();
        ui.upgrade().unwrap().on_activate_selection(move || {
            if let Some(ui) = ui.upgrade() {
                let index = ui.get_selected_index().max(0) as usize;
                if let Some(row) = ui.get_rows().row_data(index) {
                    ui.set_status_text(format!("Selected #{} · {}", row.id, row.preview).into());
                }
            }
        });
    }

    {
        let ui = ui.as_weak();
        ui.upgrade().unwrap().on_hide_temporarily(move || {
            if let Some(window) = ui.upgrade() {
                let _ = window.hide();
                let ui = ui.clone();
                Timer::single_shot(Duration::from_secs(2), move || {
                    if let Some(window) = ui.upgrade() {
                        let _ = window.show();
                    }
                });
            }
        });
    }

    if let Ok(milliseconds) = std::env::var("TIEZ_NATIVE_POC_AUTO_EXIT_MS") {
        if let Ok(milliseconds) = milliseconds.parse::<u64>() {
            Timer::single_shot(Duration::from_millis(milliseconds), || {
                let _ = slint::quit_event_loop();
            });
        }
    }

    let auto_destroy_ms = std::env::var("TIEZ_NATIVE_POC_AUTO_DESTROY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());

    if let Some(milliseconds) = auto_destroy_ms {
        ui.show()?;
        let ui_holder = Rc::new(RefCell::new(Some(ui)));
        let holder_for_timer = ui_holder.clone();
        Timer::single_shot(Duration::from_millis(milliseconds), move || {
            if let Some(window) = holder_for_timer.borrow_mut().take() {
                let _ = window.hide();
            }
        });
        let result = slint::run_event_loop_until_quit();
        drop(ui_holder);
        return result;
    }

    if let Ok(milliseconds) = std::env::var("TIEZ_NATIVE_POC_AUTO_HIDE_MS") {
        if let Ok(milliseconds) = milliseconds.parse::<u64>() {
            let weak_ui = ui.as_weak();
            Timer::single_shot(Duration::from_millis(milliseconds), move || {
                if let Some(window) = weak_ui.upgrade() {
                    let _ = window.hide();
                }
            });
            ui.show()?;
            return slint::run_event_loop_until_quit();
        }
    }

    ui.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_preview_source_and_kind_case_insensitively() {
        let rows = make_rows(30);
        assert!(filtered_rows(&rows, "screenshot")
            .iter()
            .all(|row| row.kind == "image"));
        assert_eq!(filtered_rows(&rows, "BROWSER").len(), 15);
        assert!(!filtered_rows(&rows, "file").is_empty());
    }

    #[test]
    fn empty_search_preserves_all_rows_and_whitespace() {
        let rows = make_rows(12);
        assert_eq!(filtered_rows(&rows, "  ").len(), rows.len());
        assert!(rows
            .iter()
            .any(|row| row.preview.contains("preserved whitespace")));
    }
}
