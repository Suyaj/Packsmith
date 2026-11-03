//! Deploy transport integration tests against a REAL SSH server (ticket 10).
//!
//! Network-free by default: they self-skip unless the same environment as
//! `sshd_integration_test.rs` (ticket 04) is provided:
//!
//! ```text
//! PACKSMITH_SSH_TEST_HOST=127.0.0.1
//! PACKSMITH_SSH_TEST_PORT=2222
//! PACKSMITH_SSH_TEST_USER=wyy
//! # one of:
//! PACKSMITH_SSH_TEST_PASSWORD=...
//! PACKSMITH_SSH_TEST_KEY=/path/to/private/key   (+ optional
//! PACKSMITH_SSH_TEST_KEY_PASSPHRASE=...)
//! ```
//!
//! What they cover end to end against real sshd:
//! - unknown host key is REJECTED by deploys (no interactive accept)
//! - with the stored fingerprint: SFTP upload under the ORIGINAL file name,
//!   overwrite of an existing file, byte-identical content
//! - exec of a serial command group (as the login user), output + exit code
//! - non-zero exit surfaces as ExecOutput{exit_code != 0}

use packsmith_lib::deploy::engine::{DeployTransport, ExecOutput, TransportError};
use packsmith_lib::deploy::transport::SshTransport;
use packsmith_lib::server::repo::Credential;

struct SshEnv {
    host: String,
    port: u16,
    username: String,
    credential: Option<Credential>,
}

fn ssh_env() -> Option<SshEnv> {
    let host = std::env::var("PACKSMITH_SSH_TEST_HOST").ok()?;
    let credential = if let Ok(password) = std::env::var("PACKSMITH_SSH_TEST_PASSWORD") {
        Some(Credential::Password { password })
    } else {
        std::env::var("PACKSMITH_SSH_TEST_KEY")
            .ok()
            .map(|path| Credential::PrivateKey {
                passphrase: std::env::var("PACKSMITH_SSH_TEST_KEY_PASSPHRASE").ok(),
                path,
            })
    }?;
    Some(SshEnv {
        host,
        port: std::env::var("PACKSMITH_SSH_TEST_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(22),
        username: std::env::var("PACKSMITH_SSH_TEST_USER").ok()?,
        credential: Some(credential),
    })
}

fn conn(env: &SshEnv, fingerprint: Option<String>) -> packsmith_lib::deploy::engine::ConnInfo {
    packsmith_lib::deploy::engine::ConnInfo {
        server_id: 0,
        host: env.host.clone(),
        port: env.port,
        username: env.username.clone(),
        credential: env.credential.clone().unwrap(),
        fingerprint,
    }
}

#[tokio::test]
async fn deploy_transport_end_to_end() {
    let Some(env) = ssh_env() else { return };
    // 1. Deploys never accept an unknown host key.
    let err = match SshTransport::connect(&conn(&env, None)).await {
        Err(e) => e,
        Ok(_) => panic!("deploys must reject an unknown host key"),
    };
    let fingerprint = match err {
        TransportError::HostKeyUnknown { fingerprint } => fingerprint,
        other => panic!("expected HostKeyUnknown, got {other:?}"),
    };

    // 2. With the stored fingerprint: connect + auth succeed.
    let mut transport = SshTransport::connect(&conn(&env, Some(fingerprint)))
        .await
        .expect("connect with accepted fingerprint");

    // Unique sandbox on the remote, cleaned up at the end.
    let dir = format!(
        "/tmp/packsmith-deploy-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    // 3. Upload under the ORIGINAL file name; the transport creates the
    //    upload dir, overwriting any existing file.
    let payload = format!("packsmith deploy test {}", line!()).into_bytes();
    let local_dir = tempfile::tempdir().unwrap();
    let artifact_name = "app-service-1.0.jar";
    let local_path = local_dir.path().join(artifact_name);
    std::fs::write(&local_path, &payload).unwrap();

    let mut progress_seen = false;
    transport
        .upload(
            &local_path,
            &dir,
            artifact_name,
            &mut |transferred, total| {
                progress_seen = true;
                assert_eq!(total as usize, payload.len());
                transferred <= total
            },
        )
        .await
        .expect("upload succeeds");

    // 4. Overwrite with different content — same name must replace it.
    let payload2 = b"second version".to_vec();
    std::fs::write(&local_path, &payload2).unwrap();
    transport
        .upload(&local_path, &dir, artifact_name, &mut |_, _| true)
        .await
        .expect("overwrite upload succeeds");

    // 5. Exec a serial command group as one unit; verify original
    //    filename, overwritten content, and byte count.
    let group = "echo start && cat FILE && wc -c < FILE && echo done"
        .replace("FILE", &format!("{dir}/{artifact_name}"));
    let out: ExecOutput = transport.exec(&group).await.expect("exec succeeds");
    assert_eq!(out.exit_code, 0, "output: {}", out.combined);
    assert!(out.combined.contains("start"));
    assert!(out.combined.contains("done"));
    // The remote file must hold the overwritten bytes under the same name.
    assert!(
        out.combined.contains("second version"),
        "remote file must match the overwritten bytes, got {}",
        out.combined
    );
    assert!(out.combined.contains(&payload2.len().to_string()));
    assert!(progress_seen, "upload progress must be reported");

    // 6. A failing command surfaces its non-zero exit code and output.
    let out = transport
        .exec("echo before-failure && exit 3")
        .await
        .unwrap();
    assert_eq!(out.exit_code, 3);
    assert!(out.combined.contains("before-failure"));

    // 7. exec of `&&` chain: first failure must stop the chain.
    let out = transport.exec("false && echo must-not-run").await.unwrap();
    assert_ne!(out.exit_code, 0);
    assert!(!out.combined.contains("must-not-run"));

    transport
        .exec(&format!("rm -rf {dir}"))
        .await
        .expect("cleanup");
}
