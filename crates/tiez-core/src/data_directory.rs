//! Shared TieZ data-directory selection policy.
//!
//! Both desktop frontends must resolve `clipboard.db` from the same directory.
//! Portable storage beside the executable wins over a valid `datapath.txt`
//! redirect, which in turn wins over the platform-provided default directory.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataDirectorySource {
    Default,
    Redirect,
    Portable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDataDirectory {
    pub path: PathBuf,
    pub source: DataDirectorySource,
}

pub fn resolve_data_directory(
    default_app_dir: impl AsRef<Path>,
    executable_path: Option<&Path>,
) -> ResolvedDataDirectory {
    let default_app_dir = default_app_dir.as_ref();
    let redirect_file = default_app_dir.join("datapath.txt");
    let redirected = std::fs::read_to_string(redirect_file)
        .ok()
        .map(|content| content.trim().to_owned())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.exists());

    let mut resolved = redirected
        .map(|path| ResolvedDataDirectory {
            path,
            source: DataDirectorySource::Redirect,
        })
        .unwrap_or_else(|| ResolvedDataDirectory {
            path: default_app_dir.to_path_buf(),
            source: DataDirectorySource::Default,
        });

    if let Some(executable_dir) = executable_path.and_then(Path::parent) {
        let portable_data = executable_dir.join("data");
        if portable_data.is_dir() {
            resolved = ResolvedDataDirectory {
                path: portable_data,
                source: DataDirectorySource::Portable,
            };
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tiez-data-directory-{name}-{unique}"))
    }

    #[test]
    fn default_directory_is_used_without_redirect_or_portable_storage() {
        let root = temporary_root("default");
        let default_dir = root.join("default");
        let executable = root.join("app").join("TieZ.exe");

        let resolved = resolve_data_directory(&default_dir, Some(&executable));

        assert_eq!(resolved.path, default_dir);
        assert_eq!(resolved.source, DataDirectorySource::Default);
    }

    #[test]
    fn valid_redirect_is_used_and_invalid_redirect_falls_back_to_default() {
        let root = temporary_root("redirect");
        let default_dir = root.join("default");
        let redirected_dir = root.join("redirected");
        std::fs::create_dir_all(&default_dir).unwrap();
        std::fs::create_dir_all(&redirected_dir).unwrap();
        std::fs::write(
            default_dir.join("datapath.txt"),
            format!("  {}  ", redirected_dir.display()),
        )
        .unwrap();

        let redirected = resolve_data_directory(&default_dir, None);
        assert_eq!(redirected.path, redirected_dir);
        assert_eq!(redirected.source, DataDirectorySource::Redirect);

        std::fs::write(
            default_dir.join("datapath.txt"),
            root.join("missing").display().to_string(),
        )
        .unwrap();
        let fallback = resolve_data_directory(&default_dir, None);
        assert_eq!(fallback.path, default_dir);
        assert_eq!(fallback.source, DataDirectorySource::Default);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn portable_storage_overrides_a_valid_redirect() {
        let root = temporary_root("portable");
        let default_dir = root.join("default");
        let redirected_dir = root.join("redirected");
        let executable_dir = root.join("app");
        let portable_dir = executable_dir.join("data");
        std::fs::create_dir_all(&default_dir).unwrap();
        std::fs::create_dir_all(&redirected_dir).unwrap();
        std::fs::create_dir_all(&portable_dir).unwrap();
        std::fs::write(
            default_dir.join("datapath.txt"),
            redirected_dir.display().to_string(),
        )
        .unwrap();

        let resolved = resolve_data_directory(
            &default_dir,
            Some(&executable_dir.join("Tiez.WinUIProbe.exe")),
        );

        assert_eq!(resolved.path, portable_dir);
        assert_eq!(resolved.source, DataDirectorySource::Portable);
        std::fs::remove_dir_all(root).unwrap();
    }
}
