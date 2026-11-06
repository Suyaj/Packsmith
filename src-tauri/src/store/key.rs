//! Encryption key management (ADR-0002 core, key storage superseded by
//! ADR-0003): the key is auto-generated on first run. As of ticket 16 it
//! lives in a key file `<appdata>/db.key` (0600, current user only) instead
//! of the OS keychain — no more keychain prompts on startup.
//!
//! The `keyring` dependency is retained **deliberately**: it powers the
//! one-time migration path (resolution step ②). Users upgrading from earlier
//! builds have their key in the OS keychain; dropping the dependency before
//! that key has been migrated would brick their existing database. This is
//! migration-only code — once `db.key` exists, no runtime path touches the
//! keychain (resolution step ① short-circuits before it).
//!
//! The keychain interaction is isolated behind [`KeySource`]; tests exercise
//! the resolution logic with fakes, never the real keychain.

use std::path::{Path, PathBuf};

use rand::TryRngCore;

use super::StoreError;

/// Name of the key file inside the app data directory.
pub const KEY_FILE_NAME: &str = "db.key";

/// The raw key as 64 hex chars (32 random bytes from the OS CSPRNG).
pub fn generate_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG is unavailable — cannot generate an encryption key");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `key` is a well-formed key (64 hex chars).
fn is_valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reads and writes the key file `<appdata>/db.key`.
///
/// The file is the *authoritative* key store. A present-but-invalid file is
/// an error, NEVER treated as absent — silently regenerating would brick the
/// existing database (same rule as the old keychain errors).
pub struct FileKeySource {
    path: PathBuf,
}

impl FileKeySource {
    /// Targets `<app_data_dir>/db.key`.
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            path: app_data_dir.join(KEY_FILE_NAME),
        }
    }

    /// The key file's full path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Some(key)` if the file exists and holds a valid key; `None` if it is
    /// absent. A corrupt/empty file is [`StoreError::InvalidKey`] — never
    /// "no key".
    pub fn load(&self) -> Result<Option<String>, StoreError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let key = text.trim();
                if !is_valid_key(key) {
                    return Err(StoreError::InvalidKey);
                }
                Ok(Some(key.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the key file atomically (temp file + rename) with 0600
    /// permissions on unix; best-effort permissions elsewhere (Windows ACLs
    /// in the user profile already restrict to the current user).
    pub fn store(&self, key: &str) -> Result<(), StoreError> {
        assert!(is_valid_key(key), "refusing to store a malformed key");
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file_name = self
            .path
            .file_name()
            .expect("key path always has a file name")
            .to_os_string();
        let tmp = self.path.with_file_name({
            let mut n = file_name;
            n.push(".tmp");
            n
        });
        std::fs::write(&tmp, key)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Abstraction over the *legacy* key location (OS keychain) — migration-only.
/// Tests substitute fakes; production uses [`KeyringKeySource`].
pub trait KeySource: Send + Sync {
    /// Returns the stored key, or `None` when the vault has no entry. Any
    /// real failure (access denied, locked keychain, ...) must be an `Err` —
    /// a locked keychain must never be treated as "no key", which would
    /// regenerate the key and make existing data unreadable.
    fn load(&self) -> Result<Option<String>, StoreError>;
    /// Removes the stored key (migration cleanup). Best-effort: the caller
    /// tolerates and logs failures — a leftover keychain entry is harmless.
    fn delete(&self) -> Result<(), StoreError>;
}

/// The production legacy key source: OS keychain via `keyring`.
pub struct KeyringKeySource {
    service: String,
    account: String,
}

impl KeyringKeySource {
    pub fn new() -> Self {
        Self {
            service: "packsmith".to_string(),
            account: "db-key".to_string(),
        }
    }
}

impl Default for KeyringKeySource {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySource for KeyringKeySource {
    fn load(&self) -> Result<Option<String>, StoreError> {
        let entry = keyring::v1::Entry::new(&self.service, &self.account)?;
        match entry.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self) -> Result<(), StoreError> {
        let entry = keyring::v1::Entry::new(&self.service, &self.account)?;
        entry.delete_credential()?;
        Ok(())
    }
}

/// Startup key resolution (ADR-0003):
///
/// ① db.key exists → use it (the keychain is never touched);
/// ② no db.key → legacy keychain has the old key → write db.key (0600) and
///    delete the keychain entry (best-effort) → use that key, so existing
///    databases stay readable;
/// ③ neither → generate a fresh key → write db.key (fresh install).
pub fn resolve_key(
    file: &FileKeySource,
    legacy: Option<&dyn KeySource>,
) -> Result<String, StoreError> {
    if let Some(key) = file.load()? {
        return Ok(key);
    }
    if let Some(legacy) = legacy
        && let Some(key) = legacy.load()?
    {
        file.store(&key)?;
        if let Err(e) = legacy.delete() {
            eprintln!("packsmith: could not delete migrated keychain entry: {e}");
        }
        return Ok(key);
    }
    let key = generate_key();
    file.store(&key)?;
    Ok(key)
}
