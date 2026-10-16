//! Same-source clipboard content processing, with explicit host resource context.
//! No Tauri state, browser renderer, capture listener or production-data discovery.
#![cfg(windows)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
pub use tiez_storage::{database, domain};
pub mod capture;
pub mod write;
pub mod preview;
pub mod document;
#[path = "../../../src-tauri/src/services/clipboard/write_helpers.rs"]
mod write_helpers;
#[path = "../../../src-tauri/src/services/clipboard/capture_helpers.rs"]
mod capture_helpers;
#[path = "../../../src-tauri/src/services/clipboard/data.rs"]
pub mod data;
#[path = "../../../src-tauri/src/services/clipboard/processing.rs"]
pub mod processing;
use std::sync::atomic::AtomicU64;
pub static LAST_APP_SET_HASH: AtomicU64 = AtomicU64::new(0);
pub static LAST_APP_SET_HASH_ALT: AtomicU64 = AtomicU64::new(0);
pub static LAST_APP_SET_IMAGE_VISUAL_HASH: AtomicU64 = AtomicU64::new(0);
pub static LAST_APP_SET_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
#[path = "../../../src-tauri/src/services/local_image_resource.rs"]
pub(crate) mod local_image_resource;
#[path = "../../../src-tauri/src/services/clipboard/utils.rs"]
pub mod utils;

pub mod services {
    pub(crate) use crate::local_image_resource;
}
pub mod infrastructure {
    pub mod windows_api {
        pub use tiez_native_platform::clipboard_formats as win_clipboard;
        pub use tiez_native_platform::source_app as window_tracker;
    }
}

/// Preserve the production standalone-image HTML classification for capture callers.
pub fn is_standalone_image_html(text: &str, html: &str) -> bool {
    utils::is_standalone_image_html(text, html)
}

thread_local! {
    static IMAGE_ROOTS: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}
mod image_roots {
    pub(super) fn current_clipboard_image_roots() -> Vec<std::path::PathBuf> {
        super::IMAGE_ROOTS
            .with(|roots| roots.borrow().clone())
            .unwrap_or_else(|| vec![std::env::temp_dir()])
    }
}
struct RestoreRoots(Option<Vec<PathBuf>>);
impl Drop for RestoreRoots {
    fn drop(&mut self) {
        IMAGE_ROOTS.with(|roots| {
            roots.replace(self.0.take());
        });
    }
}

/// Run a synchronous processing pass with roots for this core's explicit data
/// directory. Nested calls and panics restore the previous scope. Context stays on
/// this worker thread and never leaks between core handles or thread-pool jobs.
/// Returned futures must not be used to defer processing outside this scope.
pub fn with_data_directory<T>(directory: &Path, process: impl FnOnce() -> T) -> T {
    let previous = IMAGE_ROOTS
        .with(|roots| roots.replace(Some(local_image_resource::clipboard_image_roots(directory))));
    let _restore = RestoreRoots(previous);
    process()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_context_is_nested_thread_local_and_unwinds() {
        let outer = Path::new("C:/TieZ-isolated-outer");
        let inner = Path::new("C:/TieZ-isolated-inner");
        let initial = image_roots::current_clipboard_image_roots();
        with_data_directory(outer, || {
            assert_eq!(
                image_roots::current_clipboard_image_roots(),
                local_image_resource::clipboard_image_roots(outer)
            );
            let failed = std::panic::catch_unwind(|| {
                with_data_directory(inner, || {
                    assert_eq!(
                        image_roots::current_clipboard_image_roots(),
                        local_image_resource::clipboard_image_roots(inner)
                    );
                    panic!("synthetic processing failure");
                })
            });
            assert!(failed.is_err());
            assert_eq!(
                image_roots::current_clipboard_image_roots(),
                local_image_resource::clipboard_image_roots(outer)
            );
            assert_eq!(
                std::thread::spawn(image_roots::current_clipboard_image_roots)
                    .join()
                    .unwrap(),
                initial
            );
        });
        assert_eq!(image_roots::current_clipboard_image_roots(), initial);
    }
}

/// Scope used by the core for stored images and preview fallback resources.
pub fn clipboard_image_roots(directory: &Path) -> Vec<PathBuf> {
    local_image_resource::clipboard_image_roots(directory)
}
