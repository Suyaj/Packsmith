//! Encrypted single-database store (ADR-0002).
//!
//! One SQLCipher database holds all structured data and secrets. The
//! encryption key is auto-generated on first run and kept in the OS keychain
//! (`key` module). This module is the shared data-access and migration entry
//! point for all later tickets.

/// Encryption key lifecycle: generation + OS-keychain storage.
pub mod key;
mod migrations;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

pub use key::{KeySource, KeyringKeySource};

/// Schema version after all declared migrations have been applied.
pub const LATEST_SCHEMA_VERSION: i32 = migrations::LATEST_SCHEMA_VERSION;

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Keyring(keyring::Error),
    /// The database file exists but could not be decrypted with the given key.
    DecryptionFailed,
    /// The key is not 64 hex chars (a raw 32-byte key).
    InvalidKey,
    /// SQLCipher is not linked into this build (plain SQLite instead) — refuse to open.
    SqlCipherMissing,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "io error: {e}"),
            StoreError::Sql(e) => write!(f, "sqlite error: {e}"),
            StoreError::Keyring(e) => write!(f, "keychain error: {e}"),
            StoreError::DecryptionFailed => {
                write!(f, "database could not be decrypted with the provided key")
            }
            StoreError::InvalidKey => {
                write!(f, "invalid key: expected 32 bytes as 64 hex chars")
            }
            StoreError::SqlCipherMissing => {
                write!(f, "SQLCipher is not linked into this build")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io(e) => Some(e),
            StoreError::Sql(e) => Some(e),
            StoreError::Keyring(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

impl From<keyring::Error> for StoreError {
    fn from(e: keyring::Error) -> Self {
        StoreError::Keyring(e)
    }
}

/// The single encrypted application database.
///
/// Cheap to share: wrap in an `Arc` (or hand to `tauri::manage`) and call
/// [`Store::with`] from any thread. The same connection is reused, so data
/// written through one handle is visible to all others.
pub struct Store {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// Opens (creating if needed) the encrypted database at `path`.
    ///
    /// `key_hex` is the raw 32-byte key as 64 hex chars. It is applied as a
    /// raw key (`x'...'`) — NOT a passphrase — so the stored value must be
    /// exactly what [`key`] generated.
    pub fn open(path: &Path, key_hex: &str) -> Result<Self, StoreError> {
        if key_hex.len() != 64 || !key_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::InvalidKey);
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;

        // First statement on the connection must be the key. The string form
        // `'x''<hex>'''` is SQLCipher's documented raw-key syntax (no PBKDF2),
        // avoiding the passphrase/raw-key ambiguity. (A blob literal `x'...'`
        // is not accepted by SQLite's PRAGMA-value grammar.)
        conn.execute_batch(&format!("PRAGMA key = 'x''{key_hex}''';"))?;

        // Fail fast with a clear error if this build linked plain SQLite
        // (e.g. via feature unification) instead of SQLCipher.
        let cipher: Option<String> = match conn.query_row("PRAGMA cipher_version", [], |r| r.get(0))
        {
            Ok(v) => v,
            // Plain SQLite has no such pragma and reports no rows.
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };
        if cipher.as_deref().unwrap_or_default().is_empty() {
            return Err(StoreError::SqlCipherMissing);
        }

        // Reading a page with a wrong key fails; probe so the caller gets a
        // clear "wrong key" error instead of a delayed surprise.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| StoreError::DecryptionFailed)?;

        migrations::run(&conn)?;

        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// Reports the linked SQLCipher version (sanity check for tests and the
    /// settings page).
    pub fn cipher_version(&self) -> Result<String, StoreError> {
        let conn = self.conn.lock().unwrap();
        let version: Option<String> = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
        Ok(version.unwrap_or_default())
    }

    /// Runs `f` with the underlying connection. This is THE data-access entry
    /// point shared by all later tickets — repositories are built on it, the
    /// raw connection never leaks further out.
    pub fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(f(&conn)?)
    }

    /// Opens the store using the production key source (OS keychain). This is
    /// what app startup calls; tests pass keys explicitly via [`Store::open`].
    pub fn open_default(path: &Path) -> Result<Self, StoreError> {
        let key = KeyringKeySource::new().load_or_create()?;
        Store::open(path, &key)
    }
}
