//! Integration tests for the encrypted store (ticket 02, key storage ticket 16).
//!
//! Seams under test:
//! - `Store::open(path, key_hex)` — open + key + migrations
//! - `Store::with(fn(&Connection))` — generic data access
//! - the DB file itself — encryption is observable from outside
//! - `store::key` — key-file storage, legacy-keychain migration, resolution
//!   order (file → keychain → generate)
//!
//! Tests never touch the OS keychain; `KeyringKeySource` is wired in app
//! startup only and fakes stand in via the `KeySource` seam.

use std::sync::atomic::{AtomicBool, Ordering};

use packsmith_lib::store::key;
use packsmith_lib::store::{Store, StoreError};

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

#[test]
fn open_reports_sqlcipher_is_linked() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();

    let version = store.cipher_version().unwrap();
    assert!(
        !version.is_empty(),
        "PRAGMA cipher_version must be non-empty (SQLCipher must be linked, not plain SQLite)"
    );
}

#[test]
fn open_creates_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    Store::open(&db, TEST_KEY_HEX).unwrap();

    assert!(db.exists(), "opening a fresh store must create the db file");
}

#[test]
fn open_applies_migrations_and_records_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();

    let version: i32 = store
        .with(|conn| conn.query_row("PRAGMA user_version", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(version, packsmith_lib::store::LATEST_SCHEMA_VERSION);

    // Every declared migration is recorded as applied.
    let applied: i64 = store
        .with(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='app_meta'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(applied, 1, "baseline migration must have created app_meta");
}

#[test]
fn reopening_an_old_database_applies_new_migrations_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    // First open: migrations v1..latest run.
    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let version: i32 = store
        .with(|conn| conn.query_row("PRAGMA user_version", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(version, packsmith_lib::store::LATEST_SCHEMA_VERSION);
    drop(store);

    // Reopen: idempotent, same version, data still there.
    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO app_meta (key, value) VALUES ('probe', 'ok')",
                [],
            )
        })
        .unwrap();
    drop(store);

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let value: String = store
        .with(|conn| {
            conn.query_row("SELECT value FROM app_meta WHERE key = 'probe'", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(value, "ok");
}

#[test]
fn data_persists_across_open_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO app_meta (key, value) VALUES ('greeting', '你好 Packsmith')",
                [],
            )
        })
        .unwrap();
    drop(store);

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let greeting: String = store
        .with(|conn| {
            conn.query_row(
                "SELECT value FROM app_meta WHERE key = 'greeting'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(greeting, "你好 Packsmith");
}

#[test]
fn database_file_on_disk_is_not_plaintext_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO app_meta (key, value) VALUES ('secret', 'token-123')",
                [],
            )
        })
        .unwrap();
    drop(store);

    let header = std::fs::read(&db).unwrap();
    assert!(header.len() > 32);
    // Plaintext SQLite files start with the magic string "SQLite format 3\0".
    assert_ne!(
        &header[..16],
        b"SQLite format 3\x00",
        "db file header must not be a plaintext SQLite header"
    );
    let body = String::from_utf8_lossy(&header);
    assert!(
        !body.contains("token-123"),
        "db file must not contain plaintext row data"
    );
}

#[test]
fn opening_without_the_key_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO app_meta (key, value) VALUES ('secret', 'token-123')",
                [],
            )
        })
        .unwrap();
    drop(store);

    // A fresh DB (no SQLCipher key at all) must not be able to read it.
    let err = rusqlite::Connection::open(&db).unwrap().query_row(
        "SELECT count(*) FROM sqlite_master",
        [],
        |r| r.get::<_, i64>(0),
    );
    assert!(
        err.is_err(),
        "plain sqlite (no key) must fail to read the encrypted db"
    );

    // And the store with a WRONG key must also fail.
    let wrong = format!("{:0>64}", "0");
    let err = Store::open(&db, &wrong);
    assert!(
        matches!(err, Err(StoreError::DecryptionFailed)),
        "opening with a wrong key must fail, got {err:?}"
    );
}

#[test]
fn open_rejects_malformed_keys() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    for bad in ["", "short", &"g".repeat(64), &"a".repeat(63)] {
        let err = Store::open(&db, bad).unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidKey),
            "key {bad:?} must be rejected, got {err:?}"
        );
    }
}

#[test]
fn generated_key_is_64_hex_chars() {
    let k = key::generate_key();
    assert_eq!(k.len(), 64, "32 random bytes as hex");
    assert!(
        k.chars().all(|c| c.is_ascii_hexdigit()),
        "key must be hex only, got {k}"
    );
    assert_ne!(key::generate_key(), key::generate_key(), "keys are random");
}

// ---------------------------------------------------------------------------
// Key file (ticket 16, ADR-0003)
// ---------------------------------------------------------------------------

/// A fake legacy keychain: returns the canned key (or None) and records
/// whether `delete` was called. A `panic_on_load` variant stands in for
/// "the keychain must never be touched".
struct FakeKeychain {
    key: Option<String>,
    deleted: AtomicBool,
}

impl key::KeySource for FakeKeychain {
    fn load(&self) -> Result<Option<String>, StoreError> {
        Ok(self.key.clone())
    }
    fn delete(&self) -> Result<(), StoreError> {
        self.deleted.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// A legacy source whose mere use is a bug — resolution step ① (db.key
/// exists) must never reach it.
struct PanicKeychain;

impl key::KeySource for PanicKeychain {
    fn load(&self) -> Result<Option<String>, StoreError> {
        panic!("db.key exists: the keychain must not be touched")
    }
    fn delete(&self) -> Result<(), StoreError> {
        panic!("db.key exists: the keychain must not be touched")
    }
}

#[test]
fn file_key_source_round_trips_and_reports_absence() {
    let dir = tempfile::tempdir().unwrap();
    let file = key::FileKeySource::new(dir.path());

    assert!(
        file.load().unwrap().is_none(),
        "no key file yet must read as absent"
    );

    file.store(TEST_KEY_HEX).unwrap();
    assert_eq!(file.load().unwrap().as_deref(), Some(TEST_KEY_HEX));
    assert_eq!(file.path(), &dir.path().join("db.key"));
}

#[test]
fn file_key_source_survives_trailing_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.key");
    std::fs::write(&path, format!("{TEST_KEY_HEX}\n")).unwrap();

    let file = key::FileKeySource::new(dir.path());
    assert_eq!(file.load().unwrap().as_deref(), Some(TEST_KEY_HEX));
}

#[cfg(unix)]
#[test]
fn file_key_source_is_written_with_0600_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let file = key::FileKeySource::new(dir.path());

    file.store(TEST_KEY_HEX).unwrap();

    let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "db.key must be readable by the current user only, got {:o}",
        mode & 0o777
    );
}

#[test]
fn corrupt_or_empty_key_file_is_an_error_never_regenerated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.key");

    for garbage in ["", "not a key", &"a".repeat(63), &"g".repeat(64)] {
        std::fs::write(&path, garbage).unwrap();
        let file = key::FileKeySource::new(dir.path());
        let err = key::resolve_key(&file, Some(&PanicKeychain)).unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidKey),
            "corrupt key file {garbage:?} must error, not regenerate, got {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            garbage,
            "the corrupt file must be left untouched (data safety)"
        );
    }
}

#[test]
fn fresh_install_generates_key_file_without_touching_keychain() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("packsmith.db");
    let key_file = dir.path().join("db.key");

    // No db file yet = fresh install: the keychain must not be touched even
    // though db.key does not exist either.
    let key1 = Store::open_with_legacy_source(&db, Some(&PanicKeychain))
        .unwrap()
        .cipher_version()
        .unwrap();
    assert!(!key1.is_empty());
    assert!(key_file.exists(), "fresh install must create db.key");

    // Second run: the stored key is reused (still no keychain access).
    Store::open_with_legacy_source(&db, Some(&PanicKeychain)).unwrap();
    let stored = std::fs::read_to_string(&key_file).unwrap();
    assert_eq!(stored.trim().len(), 64);
}

#[test]
fn legacy_keychain_errors_propagate_without_regenerating() {
    struct BrokenKeychain;
    impl key::KeySource for BrokenKeychain {
        fn load(&self) -> Result<Option<String>, StoreError> {
            Err(StoreError::Keyring(keyring::Error::NoDefaultStore))
        }
        fn delete(&self) -> Result<(), StoreError> {
            panic!("must not delete when reading failed")
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let file = key::FileKeySource::new(dir.path());

    let err = key::resolve_key(&file, Some(&BrokenKeychain)).unwrap_err();
    assert!(
        matches!(err, StoreError::Keyring(_)),
        "a locked keychain must propagate, not trigger regeneration, got {err:?}"
    );
    assert!(
        !file.path().exists(),
        "a failed keychain read must not create a key file"
    );
}

#[test]
fn keychain_migration_moves_key_to_file_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let file = key::FileKeySource::new(dir.path());
    let legacy = FakeKeychain {
        key: Some(TEST_KEY_HEX.to_string()),
        deleted: AtomicBool::new(false),
    };

    let key = key::resolve_key(&file, Some(&legacy)).unwrap();
    assert_eq!(key, TEST_KEY_HEX, "the migrated key must be the OLD key");
    assert_eq!(
        file.load().unwrap().as_deref(),
        Some(TEST_KEY_HEX),
        "migration must write the key to db.key"
    );
    assert!(
        legacy.deleted.load(Ordering::SeqCst),
        "migration must delete the keychain entry"
    );

    // After migration, the keychain is never touched again.
    let key2 = key::resolve_key(&file, Some(&PanicKeychain)).unwrap();
    assert_eq!(key2, TEST_KEY_HEX);
}

#[test]
fn migration_uses_old_key_so_existing_database_stays_readable() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("packsmith.db");

    // The "old install": data written with the key that currently lives in
    // the keychain (no db.key yet).
    {
        let store = Store::open(&db, TEST_KEY_HEX).unwrap();
        store
            .with(|conn| {
                conn.execute(
                    "INSERT INTO app_meta (key, value) VALUES ('user-data', '真实数据')",
                    [],
                )
            })
            .unwrap();
    }

    // Upgrade: keychain holds the old key, db.key does not exist yet.
    let legacy = FakeKeychain {
        key: Some(TEST_KEY_HEX.to_string()),
        deleted: AtomicBool::new(false),
    };
    let store = Store::open_with_legacy_source(&db, Some(&legacy)).unwrap();

    let value: String = store
        .with(|conn| {
            conn.query_row(
                "SELECT value FROM app_meta WHERE key = 'user-data'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(value, "真实数据", "user data must survive the migration");
    assert!(
        legacy.deleted.load(Ordering::SeqCst),
        "keychain entry must be cleaned up"
    );

    // Reopen through the normal path (db.key now exists).
    drop(store);
    let store = Store::open_default(&db).unwrap();
    let value: String = store
        .with(|conn| {
            conn.query_row(
                "SELECT value FROM app_meta WHERE key = 'user-data'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(value, "真实数据");
}

#[test]
fn fresh_install_via_store_creates_key_file_and_db() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("packsmith.db");

    // `None` = pretend there is no keychain at all (fresh machine).
    let store = Store::open_with_legacy_source(&db, None).unwrap();
    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO app_meta (key, value) VALUES ('probe', 'ok')",
                [],
            )
        })
        .unwrap();
    drop(store);

    let key_file = dir.path().join("db.key");
    assert!(key_file.exists(), "fresh install must create db.key");
    let stored = std::fs::read_to_string(&key_file).unwrap();
    assert_eq!(stored.trim().len(), 64, "db.key holds a 32-byte hex key");

    // Reopen: same key from the file, data still there.
    let store = Store::open_default(&db).unwrap();
    let value: String = store
        .with(|conn| {
            conn.query_row("SELECT value FROM app_meta WHERE key = 'probe'", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(value, "ok");
}
