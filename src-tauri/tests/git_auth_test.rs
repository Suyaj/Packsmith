//! 单元测试：Git HTTPS 认证的纯决策逻辑 (ticket 13)。
//!
//! 端到端的"私有仓库 clone 成功"无法在进程内密闭验证（git2 本地传输忽略
//! 凭据，真 HTTP 服务器超出测试范围），因此在此测**选择逻辑**：
//! - URL 分类（SSH 协议拒绝 / HTTPS 与本地路径放行）
//! - 凭据回调：已配置 → UserPass(用户名, 令牌)；再次被提示（凭据被拒）→
//!   报错中止而不是无限重试
//! - git2 Auth 错误 → 明确的中文可定位错误（引导去 设置 → Git 账号）

use git2::{Cred, CredentialType, ErrorClass, ErrorCode};

use packsmith_lib::git_auth::{AuthAttempt, GitCredentials, is_ssh_url};
use packsmith_lib::store::projects::ProjectError;

fn creds() -> GitCredentials {
    GitCredentials {
        username: "octocat".to_string(),
        token: "ghp_secret-token-123".to_string(),
    }
}

#[test]
fn scp_like_ssh_urls_are_rejected() {
    assert!(is_ssh_url("git@github.com:foo/bar.git"));
    assert!(is_ssh_url("git@gitlab.example.com:group/repo.git"));
}

#[test]
fn ssh_scheme_urls_are_rejected() {
    assert!(is_ssh_url("ssh://git@github.com/foo/bar.git"));
    assert!(is_ssh_url("git+ssh://git@github.com/foo/bar.git"));
}

#[test]
fn https_and_local_urls_are_allowed() {
    assert!(!is_ssh_url("https://github.com/foo/bar.git"));
    assert!(!is_ssh_url("http://github.com/foo/bar.git"));
    assert!(!is_ssh_url("/home/me/repos/bar"));
    assert!(!is_ssh_url("file:///home/me/repos/bar"));
    // Windows 盘符路径含冒号但不是 scp 语法（冒号前没有 @）。
    assert!(!is_ssh_url(r"C:\repos\bar"));
}

#[test]
fn configured_credentials_select_userpass_plaintext() {
    // Cred 本身不可内省（libgit2 不暴露密码），这里验证：配置了账号时回调
    // 产出 UserPassPlaintext 类型凭据（与用真实用户名/令牌构造的同型）。
    let attempt = AuthAttempt::new();
    let cred = attempt
        .try_credential(
            &creds(),
            None,
            Some("ignored-hint"),
            CredentialType::USER_PASS_PLAINTEXT,
        )
        .unwrap();

    let reference = Cred::userpass_plaintext("octocat", "ghp_secret-token-123").unwrap();
    assert_eq!(cred.credtype(), reference.credtype());
}

#[test]
fn second_auth_prompt_aborts_instead_of_looping() {
    // 凭据被拒时 libgit2 会再次回调；继续给同样的凭据会无限循环。
    let attempt = AuthAttempt::new();
    assert!(
        attempt
            .try_credential(&creds(), None, None, CredentialType::USER_PASS_PLAINTEXT)
            .is_ok()
    );
    let second = attempt.try_credential(&creds(), None, None, CredentialType::USER_PASS_PLAINTEXT);
    assert!(
        second.is_err(),
        "第二次提示必须报错中止（凭据已被服务器拒绝）"
    );
}

#[test]
fn git_auth_error_maps_to_git_auth_failed_code() {
    let e = git2::Error::new(ErrorCode::Auth, ErrorClass::Http, "credentials rejected");
    match packsmith_lib::store::projects::map_git_error(e) {
        ProjectError::GitAuth(_) => {}
        other => panic!("expected GitAuth, got {other:?}"),
    }
}

#[test]
fn non_auth_error_keeps_generic_clone_error() {
    let e = git2::Error::new(ErrorCode::NotFound, ErrorClass::Reference, "not found");
    match packsmith_lib::store::projects::map_git_error(e) {
        ProjectError::Clone(_) => {}
        other => panic!("expected Clone, got {other:?}"),
    }
}

#[test]
fn error_codes_are_stable_for_i18n() {
    assert_eq!(ProjectError::GitAuth("x".into()).code(), "git_auth_failed");
    assert_eq!(
        ProjectError::SshNotSupported("x".into()).code(),
        "ssh_not_supported"
    );
}

// ---------------------------------------------------------------------------
// 真网络测试（默认忽略）：对真实私有 HTTPS 仓库验证端到端认证。
// git2 本地传输忽略凭据，进程内无法密闭模拟真 HTTP 认证；用户提供真实
// 私有仓库凭据后运行：
//
//   PACKSMITH_TEST_GIT_URL=https://github.com/<owner>/<private>.git \
//   PACKSMITH_TEST_GIT_USERNAME=<user> PACKSMITH_TEST_GIT_TOKEN=<token> \
//   cargo test --test git_auth_test -- --ignored
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs real private HTTPS repo credentials via env vars"]
fn real_network_private_repo_clone_with_configured_credentials() {
    let Ok(url) = std::env::var("PACKSMITH_TEST_GIT_URL") else {
        panic!("set PACKSMITH_TEST_GIT_URL / _USERNAME / _TOKEN to run this test");
    };
    let username = std::env::var("PACKSMITH_TEST_GIT_USERNAME").unwrap();
    let token = std::env::var("PACKSMITH_TEST_GIT_TOKEN").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = packsmith_lib::store::Store::open(
        &dir.path().join("test.db"),
        "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
    )
    .unwrap();
    packsmith_lib::store::settings::set_git_username(&store, &username).unwrap();
    packsmith_lib::store::settings::set_git_token(&store, &token).unwrap();
    assert!(packsmith_lib::git_auth::credentials_from_store(&store).is_some());

    let dest = dir.path().join("private-clone");
    let err = packsmith_lib::store::projects::clone_repo(
        &url,
        &dest,
        None,
        &mut |_| {},
        &packsmith_lib::store::projects::new_cancel_flag(),
        packsmith_lib::git_auth::credentials_from_store(&store).as_ref(),
    )
    .err();
    assert!(
        err.is_none(),
        "配置了凭据的私有仓库 clone 应成功，得到: {err:?}"
    );
    assert!(dest.join(".git").exists());

    // 对照：未配置凭据时同一仓库必须失败为 git_auth_failed。
    std::fs::remove_dir_all(&dest).unwrap();
    packsmith_lib::store::settings::clear_git_account(&store).unwrap();
    let err = packsmith_lib::store::projects::clone_repo(
        &url,
        &dest,
        None,
        &mut |_| {},
        &packsmith_lib::store::projects::new_cancel_flag(),
        None,
    )
    .err()
    .expect("未配置凭据时私有仓库 clone 应失败");
    assert_eq!(err.code(), "git_auth_failed");
}
