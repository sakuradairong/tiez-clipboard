//! Cross-process ownership guard for one TieZ database.
//!
//! The production Tauri runtime and the native WinUI runtime must never write
//! the same SQLite database concurrently. On Windows this module holds a named
//! mutex for the lifetime of the owning process. Other platforms retain their
//! existing single-instance behavior until they gain a native frontend.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum DatabaseInstanceError {
    AlreadyOwned(PathBuf),
    System { database_path: PathBuf, code: u32 },
}

impl fmt::Display for DatabaseInstanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOwned(path) => write!(
                formatter,
                "another TieZ process already owns {}",
                path.display()
            ),
            Self::System {
                database_path,
                code,
            } => write!(
                formatter,
                "failed to acquire ownership for {} (Windows error {code})",
                database_path.display()
            ),
        }
    }
}

impl std::error::Error for DatabaseInstanceError {}

#[derive(Debug)]
pub struct DatabaseInstanceGuard {
    resource_key: String,
    database_path: PathBuf,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for DatabaseInstanceGuard {}
#[cfg(windows)]
unsafe impl Sync for DatabaseInstanceGuard {}

impl DatabaseInstanceGuard {
    pub fn acquire(database_path: impl AsRef<Path>) -> Result<Self, DatabaseInstanceError> {
        let database_path = normalized_database_path(database_path.as_ref());
        let resource_key = database_resource_key(&database_path);

        #[cfg(windows)]
        {
            use std::iter::once;
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;

            let mutex_name = format!("Local\\TieZ.Database.{:016x}", stable_hash(&resource_key));
            let wide_name = std::ffi::OsStr::new(&mutex_name)
                .encode_wide()
                .chain(once(0))
                .collect::<Vec<_>>();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide_name.as_ptr()) };
            if handle.is_null() {
                return Err(DatabaseInstanceError::System {
                    database_path,
                    code: unsafe { GetLastError() },
                });
            }

            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(DatabaseInstanceError::AlreadyOwned(database_path));
            }

            return Ok(Self {
                resource_key,
                database_path,
                handle,
            });
        }

        #[cfg(not(windows))]
        Ok(Self {
            resource_key,
            database_path,
        })
    }

    pub fn protects(&self, database_path: impl AsRef<Path>) -> bool {
        self.resource_key == database_resource_key(database_path.as_ref())
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }
}

#[cfg(windows)]
impl Drop for DatabaseInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

fn normalized_database_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let Some(file_name) = absolute.file_name() else {
        return absolute;
    };
    let Some(parent) = absolute.parent() else {
        return absolute;
    };

    std::fs::canonicalize(parent)
        .map(|canonical_parent| canonical_parent.join(file_name))
        .unwrap_or(absolute)
}

fn database_resource_key(path: &Path) -> String {
    let normalized = normalized_database_path(path);
    let value = normalized.to_string_lossy();

    #[cfg(windows)]
    {
        value.replace('/', "\\").to_lowercase()
    }

    #[cfg(not(windows))]
    value.into_owned()
}

#[cfg(windows)]
fn stable_hash(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_matches_the_same_normalized_database_path() {
        let path = std::env::temp_dir().join("tiez-instance-guard.db");
        let guard = DatabaseInstanceGuard::acquire(&path).unwrap();
        assert!(guard.protects(&path));
        assert_eq!(guard.database_path(), normalized_database_path(&path));
    }

    #[cfg(windows)]
    #[test]
    fn windows_guard_rejects_a_second_owner_and_releases_on_drop() {
        let path = std::env::temp_dir().join(format!(
            "tiez-instance-guard-{}-{}.db",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let first = DatabaseInstanceGuard::acquire(&path).unwrap();
        assert!(matches!(
            DatabaseInstanceGuard::acquire(&path),
            Err(DatabaseInstanceError::AlreadyOwned(_))
        ));
        drop(first);
        DatabaseInstanceGuard::acquire(path).unwrap();
    }
}
