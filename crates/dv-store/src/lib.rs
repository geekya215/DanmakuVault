//! Local storage: vault directory layout, SQLite index, atomic writes.

pub mod fsx;
pub mod migrations;
pub mod repo;
pub mod vault;

pub use fsx::{write_atomic, write_json_atomic};
pub use migrations::SCHEMA_VERSION;
pub use vault::{Vault, VaultMarker, VaultPaths, VaultStats};
