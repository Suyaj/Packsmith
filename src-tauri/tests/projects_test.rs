//! Integration tests for 项目 (Project) management (ticket 05).
//!
//! Seams under test:
//! - `store::projects` — record CRUD over `Store` (list / create git / import)
//!   with duplicate-name, duplicate-URL and duplicate-path validation.
//! - `store::projects::clone_repo` / `clone_and_register` — git2 clone into a
//!   directory, observable through the filesystem and the project list.
//!
//! Clone tests use a local fixture repository created with git2 in a temp
//! dir, so they are hermetic and never touch the network.

use std::path::Path;

use packsmith_lib::store::projects::{
    self, new_cancel_flag, CloneProgress, CloneStage, ProjectError, ProjectSource, ProjectType,
};
use packsmith_lib::store::Store;

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
}

/// Creates a small local git repository with one commit containing `README.md`
/// and returns its path.
fn fixture_repo(dir: &Path, name: &str) -> std::path::PathBuf {
    let repo_path = dir.join(name);
    let repo = git2::Repository::init(&repo_path).unwrap();
    std::fs::write(repo_path.join("README.md"), format!("# {name}\n")).unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(Path::new("README.md")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();
    repo_path
}

// ---------------------------------------------------------------------------
// Record CRUD
// ---------------------------------------------------------------------------

#[test]
fn creating_a_git_project_registers_it_in_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    projects::create_git_record(
        &store,
        "demo-app",
        "https://example.com/demo-app.git",
        "/tmp/Packsmith/workspace/demo-app",
        ProjectType::Maven,
    )
    .unwrap();

    let all = projects::list(&store).unwrap();
    assert_eq!(all.len(), 1, "the created project must be listed");
    let p = &all[0];
    assert_eq!(p.name, "demo-app");
    assert_eq!(p.source, ProjectSource::Git);
    assert_eq!(p.url.as_deref(), Some("https://example.com/demo-app.git"));
    assert_eq!(p.path, "/tmp/Packsmith/workspace/demo-app");
    assert_eq!(p.project_type, ProjectType::Maven);
}

#[test]
fn importing_a_local_path_registers_it_in_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fixture_repo(dir.path(), "existing-repo");
    let store = open_store(dir.path());

    projects::import(&store, "existing-repo", &repo, ProjectType::Maven).unwrap();

    let all = projects::list(&store).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "existing-repo");
    assert_eq!(all[0].source, ProjectSource::Local);
    assert_eq!(all[0].url, None, "imported projects have no git URL");
    // 记录的路径是 canonicalize 后的绝对路径（macOS 上 /var 是 /private/var 的符号链接）。
    assert_eq!(
        all[0].path,
        repo.canonicalize().unwrap().to_string_lossy()
    );
}

#[test]
fn importing_a_missing_path_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    let err = projects::import(
        &store,
        "ghost",
        &dir.path().join("does-not-exist"),
        ProjectType::Maven,
    )
    .unwrap_err();

    assert!(
        matches!(err, ProjectError::PathNotFound(_)),
        "missing path must be reported as PathNotFound, got {err:?}"
    );
    assert!(
        projects::list(&store).unwrap().is_empty(),
        "a rejected import must not leave a record"
    );
}

#[test]
fn importing_a_file_instead_of_a_directory_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file.txt");
    std::fs::write(&file, "x").unwrap();
    let store = open_store(dir.path());

    let err = projects::import(&store, "a-file", &file, ProjectType::Maven).unwrap_err();
    assert!(
        matches!(err, ProjectError::PathNotFound(_)),
        "a plain file is not a project directory, got {err:?}"
    );
}

#[test]
fn duplicate_project_names_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fixture_repo(dir.path(), "repo-a");
    let store = open_store(dir.path());

    projects::create_git_record(
        &store,
        "same-name",
        "https://example.com/one.git",
        "/tmp/w/same-name",
        ProjectType::Maven,
    )
    .unwrap();

    for err in [
        // via git creation
        projects::create_git_record(
            &store,
            "same-name",
            "https://example.com/other.git",
            "/tmp/w/other",
            ProjectType::Maven,
        )
        .unwrap_err(),
        // and via import
        projects::import(&store, "same-name", &repo, ProjectType::Maven).unwrap_err(),
    ] {
        assert!(
            matches!(err, ProjectError::DuplicateName(_)),
            "duplicate name must be reported as DuplicateName, got {err:?}"
        );
    }
}

#[test]
fn duplicate_git_urls_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    projects::create_git_record(
        &store,
        "first",
        "https://example.com/same.git",
        "/tmp/w/first",
        ProjectType::Maven,
    )
    .unwrap();

    let err = projects::create_git_record(
        &store,
        "second",
        "https://example.com/same.git",
        "/tmp/w/second",
        ProjectType::Maven,
    )
    .unwrap_err();
    assert!(
        matches!(err, ProjectError::DuplicateUrl(_)),
        "duplicate URL must be reported as DuplicateUrl, got {err:?}"
    );
}

#[test]
fn duplicate_local_paths_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fixture_repo(dir.path(), "repo-b");
    let store = open_store(dir.path());

    projects::import(&store, "first", &repo, ProjectType::Maven).unwrap();

    let err = projects::import(&store, "second", &repo, ProjectType::Maven).unwrap_err();
    assert!(
        matches!(err, ProjectError::DuplicatePath(_)),
        "duplicate path must be reported as DuplicatePath, got {err:?}"
    );
}

#[test]
fn empty_or_path_like_names_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    for bad in ["", "  ", "a/b", "..", "a\\b"] {
        let err = projects::create_git_record(
            &store,
            bad,
            "https://example.com/x.git",
            "/tmp/w/x",
            ProjectType::Maven,
        )
        .unwrap_err();
        assert!(
            matches!(err, ProjectError::InvalidName(_)),
            "name {bad:?} must be rejected as InvalidName, got {err:?}"
        );
    }
    assert!(
        projects::list(&store).unwrap().is_empty(),
        "no records may be left behind by rejected names"
    );
}

#[test]
fn records_survive_reopening_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    {
        let store = Store::open(&db, TEST_KEY_HEX).unwrap();
        projects::create_git_record(
            &store,
            "persistent",
            "https://example.com/p.git",
            "/tmp/w/persistent",
            ProjectType::Maven,
        )
        .unwrap();
    }

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let all = projects::list(&store).unwrap();
    assert_eq!(all.len(), 1, "projects live in the encrypted store");
    assert_eq!(all[0].name, "persistent");
}

// ---------------------------------------------------------------------------
// Clone
// ---------------------------------------------------------------------------

#[test]
fn clone_and_register_clones_into_workspace_root_and_lists_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let workspace = dir.path().join("workspace");
    let store = open_store(dir.path());

    let project = projects::clone_and_register(
        &store,
        "cloned-app",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |_| {},
    )
    .unwrap();

    // 磁盘上有对应目录，且确实是一个 git 仓库，内容完整。
    let cloned = workspace.join("cloned-app");
    assert_eq!(
        project.path,
        cloned.canonicalize().unwrap().to_string_lossy(),
        "record points at the cloned directory"
    );
    assert!(cloned.join(".git").exists(), "clone produced a git repository");
    assert!(cloned.join("README.md").exists(), "working tree was checked out");

    // 项目出现在列表。
    let all = projects::list(&store).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, project.id);
    assert_eq!(all[0].name, "cloned-app");
    assert_eq!(all[0].source, ProjectSource::Git);
}

#[test]
fn clone_progress_reports_lifecycle_stages() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let workspace = dir.path().join("workspace");
    let store = open_store(dir.path());

    let mut stages = Vec::new();
    projects::clone_and_register(
        &store,
        "cloned-app",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |p: CloneProgress| stages.push(p.stage),
    )
    .unwrap();

    // 本地路径 clone 不走网络传输回调；通过网络 clone（真实 git URL）时
    // transfer_progress 会额外上报 Transferring。生命周期至少覆盖
    // Preparing → Done。
    assert!(
        stages.contains(&CloneStage::Preparing) && stages.contains(&CloneStage::Done),
        "clone must report lifecycle progress, got {stages:?}"
    );
}

#[test]
fn clone_with_an_invalid_url_fails_and_leaves_no_record_or_directory() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let store = open_store(dir.path());

    let bogus = dir.path().join("no-such-repo.git");
    let err = projects::clone_and_register(
        &store,
        "doomed",
        bogus.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |_| {},
    )
    .unwrap_err();

    assert!(
        matches!(err, ProjectError::Clone(_)),
        "invalid URL must surface as Clone error, got {err:?}"
    );
    assert!(
        projects::list(&store).unwrap().is_empty(),
        "failed clone must not register a project"
    );
    assert!(
        !workspace.join("doomed").exists(),
        "failed clone must not leave a directory behind"
    );
}

#[test]
fn clone_rejects_when_destination_directory_already_exists() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("taken")).unwrap();
    let store = open_store(dir.path());

    let err = projects::clone_and_register(
        &store,
        "taken",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |_| {},
    )
    .unwrap_err();

    assert!(
        matches!(err, ProjectError::DirectoryExists(_)),
        "existing destination must be rejected, got {err:?}"
    );
}

#[test]
fn clone_honours_cancellation_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let workspace = dir.path().join("workspace");
    let store = open_store(dir.path());

    // Cancel before starting: the clone must abort immediately.
    let cancel = new_cancel_flag();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);

    let err = projects::clone_and_register(
        &store,
        "cancelled-app",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &cancel,
        &mut |_| {},
    )
    .unwrap_err();

    assert!(
        matches!(err, ProjectError::Cancelled),
        "cancelled clone must report Cancelled, got {err:?}"
    );
    assert!(projects::list(&store).unwrap().is_empty());
    assert!(
        !workspace.join("cancelled-app").exists(),
        "cancelled clone must not leave a directory behind"
    );
}

#[test]
fn clone_validates_name_and_duplicates_before_touching_the_disk() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_repo(dir.path(), "upstream");
    let workspace = dir.path().join("workspace");
    let store = open_store(dir.path());

    // Duplicate name is rejected without cloning.
    projects::clone_and_register(
        &store,
        "first",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |_| {},
    )
    .unwrap();
    let err = projects::clone_and_register(
        &store,
        "first",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, ProjectError::DuplicateName(_)));

    // Duplicate URL is rejected without cloning.
    let err = projects::clone_and_register(
        &store,
        "second",
        fixture.to_str().unwrap(),
        workspace.to_str().unwrap(),
        ProjectType::Maven,
        &new_cancel_flag(),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, ProjectError::DuplicateUrl(_)));

    assert_eq!(projects::list(&store).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Last-used branch persistence (ticket 06: 打包分支选择)
// ---------------------------------------------------------------------------

#[test]
fn last_used_branch_is_persisted_and_survives_reopening_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    {
        let store = Store::open(&db, TEST_KEY_HEX).unwrap();
        let p = projects::create_git_record(
            &store,
            "demo-app",
            "https://example.com/demo-app.git",
            "/tmp/w/demo-app",
            ProjectType::Maven,
        )
        .unwrap();
        assert_eq!(
            p.last_branch, None,
            "a fresh project has no last-used branch"
        );
        projects::set_last_branch(&store, p.id, "release/1.0").unwrap();
    }

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let all = projects::list(&store).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].last_branch.as_deref(),
        Some("release/1.0"),
        "the last-used branch must be remembered across restarts"
    );
}

// ---------------------------------------------------------------------------
// Workspace root expansion
// ---------------------------------------------------------------------------

#[test]
fn workspace_root_tilde_is_expanded_to_the_home_directory() {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap();
    assert_eq!(
        projects::expand_workspace_root("~/Packsmith/workspace"),
        Path::new(&home).join("Packsmith/workspace")
    );
    assert_eq!(
        projects::expand_workspace_root("/absolute/path"),
        Path::new("/absolute/path")
    );
}

// ---------------------------------------------------------------------------
// Build command persistence (ticket 07)
// ---------------------------------------------------------------------------

#[test]
fn new_project_defaults_to_the_standard_build_command() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fixture_repo(dir.path(), "cmd-repo");
    let store = open_store(dir.path());

    projects::import(&store, "cmd-repo", &repo, ProjectType::Maven).unwrap();

    let p = &projects::list(&store).unwrap()[0];
    assert_eq!(p.build_command, projects::DEFAULT_BUILD_COMMAND);
    assert_eq!(p.build_command, "mvnd package -DskipTests");
}

#[test]
fn build_command_can_be_edited_and_persists_on_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fixture_repo(dir.path(), "edit-cmd-repo");
    let store = open_store(dir.path());

    let id = projects::import(&store, "edit-cmd-repo", &repo, ProjectType::Maven).unwrap().id;

    projects::set_build_command(&store, id, "mvnd package -DskipTests -Pfast").unwrap();

    let p = projects::get(&store, id).unwrap().unwrap();
    assert_eq!(p.build_command, "mvnd package -DskipTests -Pfast");

    // Reopening the store keeps the edited command.
    drop(store);
    let store = open_store(dir.path());
    let p = projects::get(&store, id).unwrap().unwrap();
    assert_eq!(p.build_command, "mvnd package -DskipTests -Pfast");
}

#[test]
fn build_command_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fixture_repo(dir.path(), "blank-cmd-repo");
    let store = open_store(dir.path());

    let id = projects::import(&store, "blank-cmd-repo", &repo, ProjectType::Maven).unwrap().id;

    assert!(matches!(
        projects::set_build_command(&store, id, "   "),
        Err(ProjectError::InvalidBuildCommand(_))
    ));
}
