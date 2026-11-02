//! Integration tests for the encrypted store (ticket 02).
//!
//! Seams under test:
//! - `Store::open(path, key_hex)` — open + key + migrations
//! - `Store::with(fn(&Connection))` — generic data access
//! - the DB file itself — encryption is observable from outside
//! - `store::key` — key generation/loading decision logic
//!
//! Tests never touch the OS keychain; `KeyringKeySource` is wired in app
//! startup only.

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

#[test]
fn key_source_returns_stored_key_on_subsequent_calls() {
    use std::cell::RefCell;

    // First run: no key in the vault -> generate, save, return it.
    // Second run: key present -> return the same one.
    let vault = RefCell::new(None);
    fn is_no_entry(e: &key::KeyError) -> bool {
        *e == key::KeyError::NoEntry
    }

    let first = key::load_or_create(
        || vault.borrow_mut().clone().ok_or(key::KeyError::NoEntry),
        |k: &str| {
            *vault.borrow_mut() = Some(k.to_string());
            Ok(())
        },
        is_no_entry,
    )
    .unwrap();
    assert_eq!(first.len(), 64);
    assert!(vault.borrow().is_some(), "generated key must be saved");

    let second = key::load_or_create(
        || vault.borrow_mut().clone().ok_or(key::KeyError::NoEntry),
        |k: &str| {
            *vault.borrow_mut() = Some(k.to_string());
            Ok(())
        },
        is_no_entry,
    )
    .unwrap();
    assert_eq!(first, second, "second run must reuse the stored key");
}

#[test]
fn key_source_propagates_keychain_errors() {
    let get = || -> Result<String, key::KeyError> {
        Err(key::KeyError::Platform("access denied".into()))
    };
    let set = |_: &str| -> Result<(), key::KeyError> {
        panic!("must not write when reading failed with a real error")
    };

    let err = key::load_or_create(get, set, |e| *e == key::KeyError::NoEntry).unwrap_err();
    assert_eq!(
        err,
        key::KeyError::Platform("access denied".into()),
        "real keychain failures must propagate, not trigger regeneration"
    );
}
