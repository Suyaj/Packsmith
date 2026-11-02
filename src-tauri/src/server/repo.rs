//! Server records: persistence over the encrypted store.
//!
//! All access goes through [`Store::with`]; the connection never leaks
//! further out. Credentials live as ordinary columns — the SQLCipher layer
//! (ADR-0002) is what makes them unreadable outside the app.

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::Store;

/// How a server authenticates: a private key (with optional passphrase)
/// or a password. Exactly one variant is persisted per row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
    Password {
        password: String,
    },
}

impl Credential {
    const KEY_METHOD: &'static str = "private_key";
    const PASSWORD_METHOD: &'static str = "password";

    fn from_row(
        method: &str,
        password: Option<String>,
        key_path: Option<String>,
        key_passphrase: Option<String>,
    ) -> Result<Self, RepoError> {
        match method {
            Self::PASSWORD_METHOD => Ok(Credential::Password {
                password: password.ok_or(RepoError::CorruptCredential)?,
            }),
            Self::KEY_METHOD => Ok(Credential::PrivateKey {
                path: key_path.ok_or(RepoError::CorruptCredential)?,
                passphrase: key_passphrase,
            }),
            other => Err(RepoError::UnknownAuthMethod(other.to_string())),
        }
    }
}

/// A new server, not yet persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewServer {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: Credential,
}

/// A persisted server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub id: i64,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: Credential,
}

#[derive(Debug)]
pub enum RepoError {
    Sql(rusqlite::Error),
    Store(crate::store::StoreError),
    CorruptCredential,
    UnknownAuthMethod(String),
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Sql(e) => write!(f, "sqlite error: {e}"),
            RepoError::Store(e) => write!(f, "store error: {e}"),
            RepoError::CorruptCredential => write!(f, "stored credential is incomplete"),
            RepoError::UnknownAuthMethod(m) => write!(f, "unknown auth method: {m}"),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<rusqlite::Error> for RepoError {
    fn from(e: rusqlite::Error) -> Self {
        RepoError::Sql(e)
    }
}

impl From<crate::store::StoreError> for RepoError {
    fn from(e: crate::store::StoreError) -> Self {
        RepoError::Store(e)
    }
}

const SELECT_COLUMNS: &str =
    "id, host, port, username, auth_method, password, private_key_path, private_key_passphrase";

fn raw_to_server(
    (id, host, port, username, method, password, key_path, key_passphrase): RawRow,
) -> Result<Server, RepoError> {
    let credential =
        Credential::from_row(&method, password, key_path, key_passphrase)?;
    Ok(Server {
        id,
        host,
        port: port.try_into().unwrap_or(22),
        username,
        credential,
    })
}

fn insert(conn: &Connection, s: &NewServer) -> rusqlite::Result<i64> {
    let (method, password, key_path, key_passphrase) = credential_columns(&s.credential);
    conn.execute(
        "INSERT INTO servers (host, port, username, auth_method, password,
                              private_key_path, private_key_passphrase)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![s.host, s.port as i64, s.username, method, password, key_path, key_passphrase],
    )?;
    Ok(conn.last_insert_rowid())
}

fn credential_columns(c: &Credential) -> (&'static str, Option<String>, Option<String>, Option<String>) {
    match c {
        Credential::PrivateKey { path, passphrase } => {
            (Credential::KEY_METHOD, None, Some(path.clone()), passphrase.clone())
        }
        Credential::Password { password } => {
            (Credential::PASSWORD_METHOD, Some(password.clone()), None, None)
        }
    }
}

type RawRow = (
    i64,
    String,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// All servers, oldest first.
pub fn list(store: &Store) -> Result<Vec<Server>, RepoError> {
    Ok(store.with(|conn| {
        let mut stmt =
            conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM servers ORDER BY id"))?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<RawRow>>>()
    })?
    .into_iter()
    .map(raw_to_server)
    .collect::<Result<Vec<_>, _>>()?)
}

/// Inserts a server and returns the persisted record (with its id).
pub fn create(store: &Store, server: &NewServer) -> Result<Server, RepoError> {
    let id = store.with(|conn| insert(conn, server))??;
    Ok(Server {
        id,
        host: server.host.clone(),
        port: server.port,
        username: server.username.clone(),
        credential: server.credential.clone(),
    })
}

/// Replaces all fields of the server with the given id. The id is taken from
/// `server.id`; the row must exist.
pub fn update(store: &Store, server: &Server) -> Result<Server, RepoError> {
    let (method, password, key_path, key_passphrase) = credential_columns(&server.credential);
    let changed = store.with(|conn| {
        conn.execute(
            "UPDATE servers SET host = ?1, port = ?2, username = ?3, auth_method = ?4,
                                password = ?5, private_key_path = ?6, private_key_passphrase = ?7
             WHERE id = ?8",
            params![
                server.host,
                server.port as i64,
                server.username,
                method,
                password,
                key_path,
                key_passphrase,
                server.id
            ],
        )
    })??;
    if changed == 0 {
        return Err(RepoError::Sql(rusqlite::Error::QueryReturnedNoRows));
    }
    Ok(server.clone())
}

/// Deletes a server. Returns whether a row was actually removed.
pub fn delete(store: &Store, id: i64) -> Result<bool, RepoError> {
    let changed = store
        .with(|conn| conn.execute("DELETE FROM servers WHERE id = ?1", params![id]))??;
    Ok(changed > 0)
}

/// Looks up a stored host fingerprint for `host:port`, if the user has
/// accepted it before (accept-once-store-fingerprint, see ssh module).
pub fn known_host_fingerprint(
    store: &Store,
    host: &str,
    port: u16,
) -> Result<Option<String>, RepoError> {
    let stored = store.with(|conn| {
        conn.query_row(
            "SELECT fingerprint FROM known_hosts WHERE host = ?1 AND port = ?2",
            params![host, port as i64],
            |r| r.get::<_, String>(0),
        )
        .optional()
    })?;
    Ok(stored)
}

/// Stores (or overwrites) the accepted host fingerprint for `host:port`.
pub fn store_host_fingerprint(
    store: &Store,
    host: &str,
    port: u16,
    fingerprint: &str,
) -> Result<(), RepoError> {
    store.with(|conn| {
        conn.execute(
            "INSERT INTO known_hosts (host, port, fingerprint) VALUES (?1, ?2, ?3)
             ON CONFLICT(host, port) DO UPDATE SET fingerprint = excluded.fingerprint",
            params![host, port as i64, fingerprint],
        )?;
        Ok(())
    })?;
    Ok(())
}
