//! Integration tests for app settings persistence (ticket 11).
//!
//! Seams under test:
//! - `settings::get_close_window_behavior` / `set_close_window_behavior`
//!   over the encrypted `Store` (`app_meta` key/value table)
//! - the stored value's string form (`'minimize' | 'quit'`) — the contract
//!   the frontend command layer and the migration depend on
//!
//! Close-behavior *decision* logic lives in pure functions and is unit-tested
//! in `src/settings.rs`.

use packsmith_lib::settings::{
    CloseWindowBehavior, get_close_window_behavior, set_close_window_behavior,
};
use packsmith_lib::store::Store;

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open(path: &std::path::Path) -> Store {
    Store::open(path, TEST_KEY_HEX).unwrap()
}

#[test]
fn default_close_window_behavior_is_minimize() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    let behavior = get_close_window_behavior(&store).unwrap();

    assert_eq!(
        behavior,
        CloseWindowBehavior::Minimize,
        "spec: 最小化到托盘 is the default"
    );
}

#[test]
fn close_window_behavior_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    set_close_window_behavior(&store, CloseWindowBehavior::Quit).unwrap();
    assert_eq!(
        get_close_window_behavior(&store).unwrap(),
        CloseWindowBehavior::Quit
    );

    set_close_window_behavior(&store, CloseWindowBehavior::Minimize).unwrap();
    assert_eq!(
        get_close_window_behavior(&store).unwrap(),
        CloseWindowBehavior::Minimize
    );
}

#[test]
fn close_window_behavior_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = open(&db);
    set_close_window_behavior(&store, CloseWindowBehavior::Quit).unwrap();
    drop(store);

    let store = open(&db);
    assert_eq!(
        get_close_window_behavior(&store).unwrap(),
        CloseWindowBehavior::Quit,
        "setting must survive an app restart (encrypted store)"
    );
}

#[test]
fn stored_value_uses_the_documented_string_form() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    set_close_window_behavior(&store, CloseWindowBehavior::Quit).unwrap();
    let raw: String = store
        .with(|conn| {
            conn.query_row(
                "SELECT value FROM app_meta WHERE key = 'close_window_behavior'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(raw, "quit");

    set_close_window_behavior(&store, CloseWindowBehavior::Minimize).unwrap();
    let raw: String = store
        .with(|conn| {
            conn.query_row(
                "SELECT value FROM app_meta WHERE key = 'close_window_behavior'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(raw, "minimize");
}

#[test]
fn migration_seeds_the_default_setting_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    let raw: String = store
        .with(|conn| {
            conn.query_row(
                "SELECT value FROM app_meta WHERE key = 'close_window_behavior'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        raw, "minimize",
        "migration must seed the documented default"
    );
}

#[test]
fn parse_accepts_only_documented_values() {
    assert_eq!(
        "minimize".parse::<CloseWindowBehavior>().unwrap(),
        CloseWindowBehavior::Minimize
    );
    assert_eq!(
        "quit".parse::<CloseWindowBehavior>().unwrap(),
        CloseWindowBehavior::Quit
    );
    assert!("".parse::<CloseWindowBehavior>().is_err());
    assert!("Minimize".parse::<CloseWindowBehavior>().is_err());
    assert!("exit".parse::<CloseWindowBehavior>().is_err());
}

#[test]
fn close_action_follows_the_setting() {
    use packsmith_lib::settings::{CloseAction, close_action};

    assert_eq!(
        close_action(CloseWindowBehavior::Minimize),
        CloseAction::HideToTray,
        "default: closing the window hides it to the tray"
    );
    assert_eq!(
        close_action(CloseWindowBehavior::Quit),
        CloseAction::Quit,
        "选退出则真退出"
    );
}

// ---------------------------------------------------------------------------
// Git 账号 (ticket 13): 用户名 + 密码/访问令牌，入加密库
// ---------------------------------------------------------------------------

use packsmith_lib::store::settings::{
    clear_git_account, get_git_token, get_git_username, set_git_token, set_git_username,
};

#[test]
fn unset_git_account_reads_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    assert_eq!(get_git_username(&store).unwrap(), None);
    assert_eq!(get_git_token(&store).unwrap(), None);
}

#[test]
fn git_account_round_trips_username_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    set_git_username(&store, "octocat").unwrap();
    set_git_token(&store, "ghp_secret-token-123").unwrap();

    assert_eq!(
        get_git_username(&store).unwrap().as_deref(),
        Some("octocat")
    );
    assert_eq!(
        get_git_token(&store).unwrap().as_deref(),
        Some("ghp_secret-token-123")
    );
}

#[test]
fn git_account_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    {
        let store = open(&db);
        set_git_username(&store, "octocat").unwrap();
        set_git_token(&store, "token-1").unwrap();
    }
    let store = open(&db);
    assert_eq!(
        get_git_username(&store).unwrap().as_deref(),
        Some("octocat")
    );
    assert_eq!(get_git_token(&store).unwrap().as_deref(), Some("token-1"));
}

#[test]
fn set_git_username_trims_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    set_git_username(&store, "  octocat  ").unwrap();
    assert_eq!(
        get_git_username(&store).unwrap().as_deref(),
        Some("octocat")
    );
}

#[test]
fn clear_git_account_removes_username_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    set_git_username(&store, "octocat").unwrap();
    set_git_token(&store, "token-1").unwrap();

    clear_git_account(&store).unwrap();

    assert_eq!(get_git_username(&store).unwrap(), None);
    assert_eq!(get_git_token(&store).unwrap(), None);
}

#[test]
fn clear_git_account_is_idempotent_when_nothing_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    clear_git_account(&store).unwrap();
    assert_eq!(get_git_username(&store).unwrap(), None);
}

use packsmith_lib::git_auth::credentials_from_store;

#[test]
fn credentials_from_store_requires_both_username_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));

    // 只配用户名：不构成可用凭据。
    set_git_username(&store, "octocat").unwrap();
    assert_eq!(credentials_from_store(&store), None);

    set_git_token(&store, "ghp_x").unwrap();
    let creds = credentials_from_store(&store).expect("both set -> Some");
    assert_eq!(creds.username, "octocat");
    assert_eq!(creds.token, "ghp_x");
}

#[test]
fn credentials_from_store_is_none_when_unset() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));
    assert_eq!(credentials_from_store(&store), None);
}

#[test]
fn clear_git_account_also_clears_credentials_from_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("test.db"));
    set_git_username(&store, "octocat").unwrap();
    set_git_token(&store, "ghp_x").unwrap();

    clear_git_account(&store).unwrap();

    assert_eq!(credentials_from_store(&store), None);
}
