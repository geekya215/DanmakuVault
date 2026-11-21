//! 本地存储：vault 目录布局、SQLite 索引、原子写入。

pub mod fsx;
pub mod migrations;
pub mod vault;

pub use fsx::{write_atomic, write_json_atomic};
pub use migrations::SCHEMA_VERSION;
pub use vault::{Vault, VaultMarker, VaultPaths, VaultStats};
