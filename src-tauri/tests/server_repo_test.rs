//! Integration tests for server CRUD (ticket 04).
//!
//! Seams under test:
//! - `server::repo::{list, create, update, delete}` — persistence over `&Store`
//! - the schema created by migration v2 (`servers` table)
//!
//! Nothing here touches the network; SSH connection testing lives in
//! `ssh_test.rs`.

use packsmith_lib::server::repo::{self, Credential, NewServer, Server};

const TEST_KEY_HEX: &str =
    "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store() -> (tempfile::TempDir, packsmith_lib::store::Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let store = packsmith_lib::store::Store::open(&db, TEST_KEY_HEX).unwrap();
    (dir, store)
}

fn password_server(host: &str) -> NewServer {
    NewServer {
        host: host.to_string(),
        port: 22,
        username: "deploy".to_string(),
        credential: Credential::Password {
            password: "s3cret".to_string(),
        },
    }
}

fn key_server(host: &str) -> NewServer {
    NewServer {
        host: host.to_string(),
        port: 2222,
        username: "root".to_string(),
        credential: Credential::PrivateKey {
            path: "/home/me/id_ed25519".to_string(),
            passphrase: Some("key-pass".to_string()),
        },
    }
}

#[test]
fn migration_creates_servers_and_known_hosts_tables() {
    let (_dir, store) = open_store();

    let count: i64 = store
        .with(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'
                 AND name IN ('servers', 'known_hosts')",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(count, 2, "migration v2 must create servers and known_hosts");
}

#[test]
fn created_server_round_trips_all_fields() {
    let (_dir, store) = open_store();

    let created = repo::create(&store, &key_server("10.0.0.5")).unwrap();

    assert!(created.id > 0);
    let listed = repo::list(&store).unwrap();
    assert_eq!(listed.len(), 1);
    let s = &listed[0];
    assert_eq!(s.host, "10.0.0.5");
    assert_eq!(s.port, 2222);
    assert_eq!(s.username, "root");
    assert_eq!(
        s.credential,
        Credential::PrivateKey {
            path: "/home/me/id_ed25519".to_string(),
            passphrase: Some("key-pass".to_string()),
        }
    );
}

#[test]
fn password_credential_round_trips() {
    let (_dir, store) = open_store();

    repo::create(&store, &password_server("web-1")).unwrap();

    let listed = repo::list(&store).unwrap();
    assert_eq!(
        listed[0].credential,
        Credential::Password {
            password: "s3cret".to_string()
        }
    );
}

#[test]
fn update_replaces_fields_and_keeps_id() {
    let (_dir, store) = open_store();

    let created = repo::create(&store, &password_server("web-1")).unwrap();

    let updated = repo::update(
        &store,
        &Server {
            id: created.id,
            host: "web-2".to_string(),
            port: 2200,
            username: "admin".to_string(),
            credential: Credential::PrivateKey {
                path: "/keys/k".to_string(),
                passphrase: None,
            },
        },
    )
    .unwrap();

    assert_eq!(updated.id, created.id);
    let listed = repo::list(&store).unwrap();
    assert_eq!(listed.len(), 1, "update must not create a new row");
    assert_eq!(listed[0].host, "web-2");
    assert_eq!(listed[0].port, 2200);
    assert_eq!(listed[0].username, "admin");
    assert_eq!(
        listed[0].credential,
        Credential::PrivateKey {
            path: "/keys/k".to_string(),
            passphrase: None
        }
    );
}

#[test]
fn delete_removes_the_row() {
    let (_dir, store) = open_store();

    let created = repo::create(&store, &password_server("web-1")).unwrap();

    let deleted = repo::delete(&store, created.id).unwrap();
    assert!(deleted);
    assert!(repo::list(&store).unwrap().is_empty());
}

#[test]
fn delete_unknown_id_returns_false() {
    let (_dir, store) = open_store();

    assert!(!repo::delete(&store, 4242).unwrap());
}

#[test]
fn servers_survive_an_app_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = packsmith_lib::store::Store::open(&db, TEST_KEY_HEX).unwrap();
    repo::create(&store, &password_server("web-1")).unwrap();
    drop(store);

    // Reopen: a fresh "app run" sees the same list.
    let store = packsmith_lib::store::Store::open(&db, TEST_KEY_HEX).unwrap();
    let listed = repo::list(&store).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].host, "web-1");
}

#[test]
fn credentials_are_not_readable_from_the_raw_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = packsmith_lib::store::Store::open(&db, TEST_KEY_HEX).unwrap();
    repo::create(&store, &password_server("web-1")).unwrap();
    repo::create(&store, &key_server("db-1")).unwrap();
    drop(store);

    let raw = std::fs::read(&db).unwrap();
    let raw_text = raw.iter().map(|&b| b as char).collect::<String>();
    for secret in ["s3cret", "key-pass", "/home/me/id_ed25519"] {
        assert!(
            !raw_text.contains(secret),
            "raw db file must not contain the plaintext secret {secret:?} (SQLCipher must be doing the encryption)"
        );
    }
}
