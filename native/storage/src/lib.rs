//! Tauri-free storage compatibility layer, compiled from the production sources.
//! Source linkage is transitional: do not fork the schema/hash/DPAPI implementations.
//! Both the legacy app and this crate execute the same source and regression cases.
#[path = "../../../src-tauri/src/database.rs"]
pub mod database;
#[path = "../../../src-tauri/src/domain/mod.rs"]
pub mod domain;
#[path = "../../../src-tauri/src/infrastructure/encryption.rs"]
pub mod shared_encryption;
#[path = "../../../src-tauri/src/infrastructure/repository/mod.rs"]
pub mod shared_repository;

pub mod infrastructure {
    pub use crate::shared_encryption as encryption;
    pub use crate::shared_repository as repository;
}
