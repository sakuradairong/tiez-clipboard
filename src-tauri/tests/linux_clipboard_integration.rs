#![cfg(target_os = "linux")]

use arboard::{Clipboard, ImageData};
use clipboard_rs::{Clipboard as _, ClipboardContext};
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn settle() {
    std::thread::sleep(Duration::from_millis(80));
}

#[test]
fn round_trips_supported_linux_clipboard_formats() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipping Linux clipboard integration test without an X11 display");
        return;
    }

    let mut owner = Clipboard::new().expect("create clipboard owner");
    let mut reader = Clipboard::new().expect("create independent clipboard reader");

    let token = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let text = format!("tiez-linux-text-{token}");
    owner.set_text(text.clone()).expect("write text");
    settle();
    assert_eq!(reader.get_text().expect("read text"), text);

    let html = format!("<strong>tiez-linux-html-{token}</strong>");
    let html_text = format!("tiez-linux-html-{token}");
    owner
        .set_html(html.clone(), Some(html_text.clone()))
        .expect("write HTML with text fallback");
    settle();
    let rich_reader = ClipboardContext::new().expect("create rich clipboard reader");
    assert_eq!(rich_reader.get_html().expect("read HTML"), html);
    assert_eq!(
        reader.get_text().expect("read HTML text fallback"),
        html_text
    );

    let rgba = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    owner
        .set_image(ImageData {
            width: 2,
            height: 2,
            bytes: Cow::Owned(rgba.clone()),
        })
        .expect("write image");
    settle();
    let image = reader.get_image().expect("read image");
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.bytes.as_ref(), rgba.as_slice());

    let test_file = std::env::temp_dir().join(format!("tiez-linux-clipboard-{token}.txt"));
    std::fs::write(&test_file, b"TieZ Linux clipboard integration test")
        .expect("create clipboard test file");
    let paths: Vec<PathBuf> = vec![test_file.clone()];
    owner.set().file_list(&paths).expect("write file list");
    settle();
    assert_eq!(reader.get().file_list().expect("read file list"), paths);

    std::fs::remove_file(test_file).expect("remove clipboard test file");
}
