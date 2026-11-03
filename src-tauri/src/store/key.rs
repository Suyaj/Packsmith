//! Encryption key management (ADR-0002): the key is auto-generated on first
//! run and stored in the OS keychain (macOS Keychain / Windows Credential
//! Manager). No user input, ever.
//!
//! The keychain interaction is isolated behind [`KeySource`]; tests exercise
//! the get-or-create decision logic with fakes, never the real keychain.

use rand::TryRngCore;

use super::StoreError;

/// Service/account identity under which the key is filed in the keychain.
const KEYCHAIN_SERVICE: &str = "packsmith";
const KEYCHAIN_ACCOUNT: &str = "db-key";

/// The raw key as 64 hex chars (32 random bytes from the OS CSPRNG).
pub fn generate_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG is unavailable — cannot generate an encryption key");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Errors from a key vault lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The vault has no key yet — first run.
    NoEntry,
    /// A real failure (access denied, locked keychain, ...). Must propagate.
    Platform(String),
}

/// Get-or-create decision logic, shared by every key source.
///
/// - key present -> return it
/// - "no entry" (per `is_no_entry`) -> generate, save, return it
/// - any other error -> propagate (a locked keychain must never silently
///   regenerate the key, which would make existing data unreadable)
///
/// Generic over the vault's error type so both the keyring adapter and tests
/// can use it without error-type gymnastics.
pub fn load_or_create<E>(
    get: impl FnOnce() -> Result<String, E>,
    set: impl FnOnce(&str) -> Result<(), E>,
    is_no_entry: impl Fn(&E) -> bool,
) -> Result<String, E> {
    match get() {
        Ok(key) => Ok(key),
        Err(e) if is_no_entry(&e) => {
            let key = generate_key();
            set(&key)?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

/// Abstraction over "where the encryption key lives". Tests substitute fakes;
/// production uses [`KeyringKeySource`].
pub trait KeySource: Send + Sync {
    fn load_or_create(&self) -> Result<String, StoreError>;
}

/// The production key source: OS keychain via `keyring`.
pub struct KeyringKeySource {
    service: String,
    account: String,
}

impl KeyringKeySource {
    pub fn new() -> Self {
        Self {
            service: KEYCHAIN_SERVICE.to_string(),
            account: KEYCHAIN_ACCOUNT.to_string(),
        }
    }
}

impl Default for KeyringKeySource {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySource for KeyringKeySource {
    fn load_or_create(&self) -> Result<String, StoreError> {
        let entry = keyring::v1::Entry::new(&self.service, &self.account)?;
        let key = load_or_create(
            || entry.get_password(),
            |k| entry.set_password(k),
            |e| matches!(e, keyring::Error::NoEntry),
        )?;
        Ok(key)
    }
}
