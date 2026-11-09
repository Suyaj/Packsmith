//! Integration tests for 打包（一）：分支选择与临时工作区 (ticket 06).
//!
//! Seams under test:
//! - `build::list_branches` — real-time branch listing (the `git ls-remote`
//!   equivalent, done in-process with git2 per ADR-0001: no PATH dependence).
//! - `build::prepare_workspace_in` — Packsmith-managed temp workspace creation,
//!   branch clone (reusing the ticket 05 clone engine) and cleanup on failure.
//!
//! All tests run against local fixture repositories (created with git2), so
//! they are hermetic and never touch the network.

use std::path::Path;
use std::sync::atomic::Ordering;

use packsmith_lib::store::Store;
use packsmith_lib::store::projects::{
    self, CloneProgress, CloneStage, ProjectError, ProjectType, new_cancel_flag,
};

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
}

/// Creates a fixture repository with branches `main` + `feature/search` and a
/// tag `v1.0`, and returns its path.
fn fixture_repo(dir: &Path, name: &str) -> std::path::PathBuf {
    let repo_path = dir.join(name);
    let repo = git2::Repository::init(&repo_path).unwrap();

    let sig = git2::Signature::now("fixture", "fixture@example.com").unwrap();
    let mut index;
    let tree;

    std::fs::write(repo_path.join("README.md"), format!("# {name}\n")).unwrap();
    index = repo.index().unwrap();
    index.add_path(Path::new("README.md")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    tree = repo.find_tree(tree_id).unwrap();
    let main_commit = repo
        .commit(Some("refs/heads/main"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    // A second branch with an extra file (so a clone of it is distinguishable).
    let parent = repo.find_commit(main_commit).unwrap();
    std::fs::write(repo_path.join("feature.txt"), "feature work\n").unwrap();
    index = repo.index().unwrap();
    index.add_path(Path::new("feature.txt")).unwrap();
    index.write().unwrap();
    let feature_tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    repo.commit(
        Some("refs/heads/feature/search"),
        &sig,
        &sig,
        "feature commit",
        &feature_tree,
        &[&parent],
    )
    .unwrap();

    // A tag: must NOT appear in the branch list.
    repo.tag(
        "v1.0",
        &repo.find_object(main_commit, None).unwrap(),
        &sig,
        "tag",
        false,
    )
    .unwrap();

    repo_path
}

// ---------------------------------------------------------------------------
// list_branches (实时 ls-remote)
// ---------------------------------------------------------------------------

#[test]
fn list_branches_returns_live_remote_branch_names_without_heads_or_tags() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");

    let mut branches =
        packsmith_lib::build::list_branches(fixture.to_str().unwrap(), None).unwrap();
    branches.sort();

    assert_eq!(
        branches,
        vec!["feature/search".to_string(), "main".to_string()],
        "refs/heads/* are listed with the prefix stripped; HEAD and tags are not branches"
    );
}

#[test]
fn list_branches_sees_branches_added_after_the_fixture_was_created() {
    // 分支列表为实时拉取：远程新建分支必须可见。
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    assert!(
        packsmith_lib::build::list_branches(fixture.to_str().unwrap(), None)
            .unwrap()
            .is_sorted()
    );

    let repo = git2::Repository::open(&fixture).unwrap();
    let head = repo
        .find_branch("main", git2::BranchType::Local)
        .unwrap()
        .into_reference()
        .peel_to_commit()
        .unwrap();
    repo.branch("release/2.0", &head, false).unwrap();

    let mut branches =
        packsmith_lib::build::list_branches(fixture.to_str().unwrap(), None).unwrap();
    branches.sort();
    assert!(branches.contains(&"release/2.0".to_string()));
}

#[test]
fn list_branches_fails_clearly_for_an_unreachable_source() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("no-such-repo.git");

    let err = packsmith_lib::build::list_branches(bogus.to_str().unwrap(), None).unwrap_err();
    assert!(
        matches!(err, ProjectError::BranchListing(_)),
        "an unreachable source must surface as BranchListing, got {err:?}"
    );
    assert_eq!(
        err.code(),
        "branch_listing_failed",
        "the frontend keys i18n on the stable error code"
    );
}

// ---------------------------------------------------------------------------
// prepare_workspace (临时工作区)
// ---------------------------------------------------------------------------

/// Registers a git project whose URL is the fixture path.
fn register_git_project(store: &Store, name: &str, url: &Path) -> projects::Project {
    projects::create_git_record(
        store,
        name,
        &url.to_string_lossy(),
        "/tmp/w/not-used-for-build",
        ProjectType::Maven,
    )
    .unwrap()
}

#[test]
fn prepare_workspace_clones_the_selected_branch_into_a_packsmith_managed_temp_dir() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let temp_root = dir.path().join("tmproot");
    let store = open_store(dir.path());
    let project = register_git_project(&store, "demo-app", &fixture);

    let mut stages = Vec::new();
    let ws = packsmith_lib::build::prepare_workspace_in(
        &store,
        project.id,
        "feature/search",
        &new_cancel_flag(),
        &mut |p: CloneProgress| stages.push(p.stage),
        &temp_root,
    )
    .unwrap();

    // 临时工作区位于指定临时根、由 Packsmith 统一命名。
    let ws_path = Path::new(&ws.path);
    assert!(ws_path.starts_with(&temp_root), "got {}", ws.path);
    assert!(
        ws_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("packsmith-build-demo-app-"),
        "temp dir is Packsmith-managed, got {}",
        ws.path
    );

    // 指定分支的代码完整：feature 分支独有文件 + 公共文件都在，
    // 且检出的正是所选分支。
    assert!(ws_path.join("README.md").exists());
    assert!(ws_path.join("feature.txt").exists());
    let cloned = git2::Repository::open(ws_path).unwrap();
    let head = cloned.head().unwrap();
    assert_eq!(
        head.shorthand(),
        Some("feature/search"),
        "the selected branch must be checked out"
    );

    // 打包生命周期进度可见（Preparing → … → Done）。
    assert!(
        stages.contains(&CloneStage::Preparing) && stages.contains(&CloneStage::Done),
        "got {stages:?}"
    );

    // 本次选择被记住：下次打开面板默认选中。
    let stored = projects::get(&store, project.id).unwrap().unwrap();
    assert_eq!(stored.last_branch.as_deref(), Some("feature/search"));

    let _ = std::fs::remove_dir_all(ws_path);
}

#[test]
fn prepare_workspace_fails_clearly_for_an_invalid_branch_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let temp_root = dir.path().join("tmproot");
    let store = open_store(dir.path());
    let project = register_git_project(&store, "demo-app", &fixture);

    let err = packsmith_lib::build::prepare_workspace_in(
        &store,
        project.id,
        "no-such-branch",
        &new_cancel_flag(),
        &mut |_| {},
        &temp_root,
    )
    .unwrap_err();

    assert!(
        matches!(err, ProjectError::Clone(_)),
        "invalid branch must surface as a clear error, got {err:?}"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&temp_root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leftovers.is_empty(),
        "failed preparation must not leave a temp workspace behind, got {leftovers:?}"
    );
    assert!(
        projects::get(&store, project.id)
            .unwrap()
            .unwrap()
            .last_branch
            .is_none(),
        "a failed preparation must not update the last-used branch"
    );
}

#[test]
fn prepare_workspace_honours_cancellation_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let temp_root = dir.path().join("tmproot");
    let store = open_store(dir.path());
    let project = register_git_project(&store, "demo-app", &fixture);

    let cancel = new_cancel_flag();
    cancel.store(true, Ordering::Relaxed);

    let err = packsmith_lib::build::prepare_workspace_in(
        &store,
        project.id,
        "main",
        &cancel,
        &mut |_| {},
        &temp_root,
    )
    .unwrap_err();

    assert!(matches!(err, ProjectError::Cancelled));
    assert_eq!(std::fs::read_dir(&temp_root).unwrap().count(), 0);
}

#[test]
fn prepare_workspace_uses_the_local_path_for_imported_projects() {
    // 本地导入的项目同样可打包：对本地路径做 ls-remote / clone（同一通道）。
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "existing-repo");
    let temp_root = dir.path().join("tmproot");
    let store = open_store(dir.path());
    let project = projects::import(&store, "existing-repo", &fixture, ProjectType::Maven).unwrap();

    let ws = packsmith_lib::build::prepare_workspace_in(
        &store,
        project.id,
        "main",
        &new_cancel_flag(),
        &mut |_| {},
        &temp_root,
    )
    .unwrap();

    assert!(Path::new(&ws.path).join("README.md").exists());
    assert_eq!(
        projects::get(&store, project.id)
            .unwrap()
            .unwrap()
            .last_branch
            .as_deref(),
        Some("main")
    );
    let _ = std::fs::remove_dir_all(&ws.path);
}

#[test]
fn prepare_workspace_rejects_an_unknown_project() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    let err = packsmith_lib::build::prepare_workspace_in(
        &store,
        4242,
        "main",
        &new_cancel_flag(),
        &mut |_| {},
        &dir.path().join("tmproot"),
    )
    .unwrap_err();
    assert!(matches!(err, ProjectError::ProjectNotFound(4242)));
}

// ---------------------------------------------------------------------------
// Git 账号与认证 (ticket 13)
// ---------------------------------------------------------------------------

#[test]
fn list_branches_rejects_ssh_urls_upfront() {
    for url in [
        "git@github.com:foo/bar.git",
        "ssh://git@github.com/foo/bar.git",
    ] {
        let err = packsmith_lib::build::list_branches(url, None).unwrap_err();
        assert!(
            matches!(err, ProjectError::SshNotSupported(_)),
            "url: {url}, got {err:?}"
        );
        assert_eq!(err.code(), "ssh_not_supported");
    }
}

#[test]
fn list_branches_with_configured_credentials_still_works_for_local_fixture() {
    // git2 本地传输忽略凭据：验证配置了凭据时分支拉取链路照常工作。
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "auth-upstream");
    let store = open_store(dir.path());
    packsmith_lib::store::settings::set_git_username(&store, "octocat").unwrap();
    packsmith_lib::store::settings::set_git_token(&store, "ghp_x").unwrap();
    let creds = packsmith_lib::git_auth::credentials_from_store(&store);

    let mut branches =
        packsmith_lib::build::list_branches(fixture.to_str().unwrap(), creds.as_ref()).unwrap();
    branches.sort();
    assert_eq!(
        branches,
        vec!["feature/search".to_string(), "main".to_string()]
    );
}
