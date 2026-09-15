//! Runtime authorization for the custom asset protocol scope.
//!
//! The static scopes in `tauri.conf.json` cover `$HOME`, `$APPDATA`,
//! `$APPLOCALDATA` and `$PROFILE`, but the data directory can be redirected
//! anywhere via `datapath.txt` or portable mode (`<install dir>/data`). Image
//! attachments externalized into such a directory would then be rejected by
//! the asset protocol (`convertFileSrc` → 403) and previews disappear after
//! every history reload. This module authorizes exactly the two data
//! subdirectories that hold protocol-served images — nothing broader.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use tauri::{Manager, Runtime};

/// Data subdirectories whose files are served to the webview through the
/// asset protocol. Both are flat directories written by this app only.
pub const DATA_ASSET_SUBDIRS: [&str; 2] = ["attachments", "emoji_favorites"];
const CUSTOM_BACKGROUND_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

pub(crate) fn data_asset_dirs(data_dir: &Path) -> Vec<PathBuf> {
    DATA_ASSET_SUBDIRS
        .iter()
        .map(|folder| data_dir.join(folder))
        .collect()
}

/// Allows the data directory's image subdirectories in the asset protocol
/// scope. Idempotent: re-allowing an already allowed directory is a no-op.
pub fn authorize_data_asset_scope<R: Runtime>(app: &tauri::AppHandle<R>, data_dir: &Path) {
    let scope = app.asset_protocol_scope();
    for dir in data_asset_dirs(data_dir) {
        if let Err(error) = scope.allow_directory(&dir, false) {
            crate::info!(
                ">>> [ASSET SCOPE] Failed to authorize {}: {error}",
                dir.display()
            );
        }
    }
}

/// Forbids a previously authorized data directory's image subdirectories.
/// Called when the data directory moves so the old grant does not outlive
/// the switch for the remainder of the process lifetime.
pub fn retire_data_asset_scope<R: Runtime>(app: &tauri::AppHandle<R>, data_dir: &Path) {
    let scope = app.asset_protocol_scope();
    for dir in data_asset_dirs(data_dir) {
        let _ = scope.forbid_directory(&dir, false);
    }
}

/// Allows the configured custom background image file in the asset protocol
/// scope. The file picker grants this only for the process lifetime, so a
/// background stored outside the static scope ($HOME/$APPDATA/...) would be
/// rejected with 403 after every restart. Fail closed: only an existing image
/// file may be granted, and authorization errors reach the caller.
pub fn authorize_custom_background_file<R: Runtime>(
    app: &tauri::AppHandle<R>,
    path: &str,
) -> AppResult<()> {
    let Some(path) = validated_custom_background_path(path)? else {
        return Ok(());
    };

    app.asset_protocol_scope()
        .allow_file(path)
        .map_err(|error| AppError::Internal(format!("无法授权自定义背景文件: {error}")))
}

fn validated_custom_background_path(path: &str) -> AppResult<Option<&Path>> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = Path::new(trimmed);
    if !path.is_file() {
        return Err(AppError::Validation(
            "自定义背景文件不存在或不可读取".to_string(),
        ));
    }
    let is_supported_image = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            CUSTOM_BACKGROUND_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
        .unwrap_or(false);
    if !is_supported_image {
        return Err(AppError::Validation(
            "自定义背景仅支持 PNG、JPG、JPEG、WEBP 或 GIF 图片".to_string(),
        ));
    }

    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::{
        authorize_custom_background_file, authorize_data_asset_scope, data_asset_dirs,
        retire_data_asset_scope, validated_custom_background_path, DATA_ASSET_SUBDIRS,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tauri::Manager;

    fn unique_test_dir(tag: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tiez {tag} #100% {}_{}",
            std::process::id(),
            unique
        ))
    }

    #[test]
    fn data_asset_dirs_are_scoped_to_image_subdirectories() {
        let base = std::path::Path::new("D:\\Apps\\TieZ #100% 中文\\data");
        let dirs = data_asset_dirs(base);

        assert_eq!(dirs.len(), DATA_ASSET_SUBDIRS.len());
        assert!(dirs.contains(&base.join("attachments")));
        assert!(dirs.contains(&base.join("emoji_favorites")));
        // The database, logs and every other data file stay outside the grant.
        assert!(!dirs.contains(&base.join("clipboard.db")));
        assert!(!dirs.contains(&base.to_path_buf()));
    }

    #[test]
    fn runtime_scope_is_limited_and_retires_previous_data_directories() {
        let base = unique_test_dir("资源范围 中文");
        let attachment = base.join("attachments").join("image.png");
        let favorite = base.join("emoji_favorites").join("动画.gif");
        let nested = base.join("attachments").join("nested").join("image.png");
        let database = base.join("clipboard.db");
        for path in [&attachment, &favorite, &nested, &database] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"synthetic fixture").unwrap();
        }

        let app = tauri::test::mock_app();
        authorize_data_asset_scope(app.handle(), &base);
        let scope = app.asset_protocol_scope();

        assert!(scope.is_allowed(&attachment));
        assert!(scope.is_allowed(&favorite));
        assert!(!scope.is_allowed(&nested));
        assert!(!scope.is_allowed(&database));

        retire_data_asset_scope(app.handle(), &base);
        assert!(!scope.is_allowed(&attachment));
        assert!(!scope.is_allowed(&favorite));

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn runtime_scope_grants_only_the_selected_background_file() {
        let dir = unique_test_dir("外部背景 中文");
        let selected = dir.join("背景 #100%.webp");
        let sibling = dir.join("其他.webp");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&selected, b"synthetic selected image").unwrap();
        fs::write(&sibling, b"synthetic sibling image").unwrap();

        let app = tauri::test::mock_app();
        authorize_custom_background_file(app.handle(), &selected.to_string_lossy()).unwrap();
        let scope = app.asset_protocol_scope();

        assert!(scope.is_allowed(&selected));
        assert!(!scope.is_allowed(&sibling));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn custom_background_validation_accepts_supported_image_with_special_characters() {
        let dir = unique_test_dir("背景验证 中文");
        let path = dir.join("背景 图.PNG");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"synthetic image fixture").unwrap();

        let path_text = path.to_string_lossy();
        let validated = validated_custom_background_path(&path_text).unwrap();

        assert_eq!(validated, Some(path.as_path()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn custom_background_validation_rejects_missing_and_non_image_files() {
        let dir = unique_test_dir("背景拒绝");
        let text_path = dir.join("private.txt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&text_path, b"not an image").unwrap();

        assert!(validated_custom_background_path(&text_path.to_string_lossy()).is_err());
        assert!(
            validated_custom_background_path(&dir.join("missing.png").to_string_lossy()).is_err()
        );
        assert_eq!(validated_custom_background_path("   ").unwrap(), None);

        fs::remove_dir_all(dir).unwrap();
    }
}
