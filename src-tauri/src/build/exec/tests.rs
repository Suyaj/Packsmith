use super::*;
use std::path::PathBuf;

fn fake_toolchain(tag: &str) -> (tempfile::TempDir, ToolchainPaths) {
    let dir = tempfile::tempdir().unwrap();
    let mvnd = dir.path().join("tools/mvnd/bin/mvnd");
    let java = dir.path().join("tools/jdk/Contents/Home/bin/java");
    std::fs::create_dir_all(&mvnd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&java.parent().unwrap()).unwrap();
    std::fs::write(&mvnd, "#!/bin/sh\n").unwrap();
    std::fs::write(&java, "fake java").unwrap();
    let _ = tag;
    (
        dir,
        ToolchainPaths {
            mvnd,
            java_home: java.parent().unwrap().parent().unwrap().to_path_buf(),
        },
    )
}

// ---------------------------------------------------------------------------
// resolve_toolchain
// ---------------------------------------------------------------------------

#[test]
fn 托管工具链解析_含contents_home的jdk() {
    let (_dir, tc) = fake_toolchain("resolve");
    let resolved = resolve_toolchain(&_dir.path().join("tools")).unwrap();
    assert_eq!(resolved, tc);
}

#[test]
fn 工具链缺失时报缺失而非回退path() {
    let dir = tempfile::tempdir().unwrap();
    // PATH 上有 mvnd（系统装了）也不允许回退（ADR-0001）。
    let err = resolve_toolchain(&dir.path()).unwrap_err();
    assert!(matches!(err, BuildError::ToolchainMissing("mvnd")));

    // 只有 mvnd、没有 jdk 也报缺失。
    let mvnd = dir.path().join("mvnd/bin/mvnd");
    std::fs::create_dir_all(mvnd.parent().unwrap()).unwrap();
    std::fs::write(&mvnd, "#!/bin/sh").unwrap();
    let err = resolve_toolchain(&dir.path()).unwrap_err();
    assert!(matches!(err, BuildError::ToolchainMissing("jdk")));
}

// ---------------------------------------------------------------------------
// 命令分词 / 校验
// ---------------------------------------------------------------------------

#[test]
fn 空命令被拒绝() {
    assert!(matches!(
        split_command("   "),
        Err(BuildError::InvalidBuildCommand(_))
    ));
    assert!(split_command("mvnd package").is_ok());
}

#[test]
fn 分词支持引号包裹的含空格参数() {
    let tokens = split_command(r#"mvnd test -Dtest="run one thing""#).unwrap();
    assert_eq!(tokens, vec!["mvnd", "test", "-Dtest=run one thing"]);
}

#[test]
fn 未闭合引号被拒绝() {
    assert!(split_command("mvnd \"oops").is_err());
}

#[test]
fn 非mvnd构建命令在保存时被拒绝() {
    assert!(validate_build_command("gradle build").is_err());
    assert!(validate_build_command("mvn -v").is_ok());
    assert!(validate_build_command("mvnd package -DskipTests").is_ok());
}

// ---------------------------------------------------------------------------
// build_spawn_spec
// ---------------------------------------------------------------------------

#[test]
fn 默认命令映射到托管mvnd并追加依赖仓库参数() {
    let (_dir, tc) = fake_toolchain("spec-default");
    let repo = PathBuf::from("/appdata/m2-repo");

    let spec = build_spawn_spec("mvnd package -DskipTests", &repo, None, &tc).unwrap();

    assert_eq!(spec.program, tc.mvnd);
    assert_eq!(
        spec.args,
        vec![
            "package",
            "-DskipTests",
            "-Dmaven.repo.local=/appdata/m2-repo",
        ]
    );
}

#[test]
fn 用户命令已含maven_repo_local时不重复追加() {
    let (_dir, tc) = fake_toolchain("spec-explicit");
    let repo = PathBuf::from("/appdata/m2-repo");

    let spec = build_spawn_spec(
        "mvnd package -Dmaven.repo.local=/appdata/m2-repo",
        &repo,
        None,
        &tc,
    )
    .unwrap();
    let count = spec
        .args
        .iter()
        .filter(|a| a.starts_with("-Dmaven.repo.local"))
        .count();
    assert_eq!(count, 1, "已有显式参数时不得重复追加");
}

#[test]
#[allow(non_snake_case)]
fn 启动规格注入JAVA_HOME并把托管bin前置到PATH() {
    let (_dir, tc) = fake_toolchain("spec-env");
    let repo = PathBuf::from("/appdata/m2-repo");

    let spec = build_spawn_spec("mvnd package", &repo, None, &tc).unwrap();

    let env: Vec<(String, String)> = spec.env.clone();
    let java_home = &env.iter().find(|(k, _)| k == "JAVA_HOME").unwrap().1;
    assert_eq!(PathBuf::from(java_home), tc.java_home);

    let path = &env.iter().find(|(k, _)| k == "PATH").unwrap().1;
    let jdk_bin = tc.java_home.join("bin").to_string_lossy().into_owned();
    let mvnd_bin = tc.mvnd.parent().unwrap().to_string_lossy().into_owned();
    assert!(
        path.starts_with(&format!("{jdk_bin}:{mvnd_bin}:")),
        "托管 JDK 与 mvnd 的 bin 必须前置到 PATH（Finder PATH 坑），实际 {path}"
    );
}

#[test]
fn 首词非mvnd的执行命令被拒绝() {
    let (_dir, tc) = fake_toolchain("spec-reject");
    let repo = PathBuf::from("/appdata/m2-repo");
    let err = build_spawn_spec("gradle assemble", &repo, None, &tc).unwrap_err();
    assert!(matches!(err, BuildError::UnsupportedBuildCommand(other) if other == "gradle"));
}

// ---------------------------------------------------------------------------
// build_spawn_spec：mvnd settings.xml 注入（ticket 18）
// ---------------------------------------------------------------------------

#[test]
fn settings_xml存在时追加_s参数_顺序在依赖仓库参数之后() {
    let (_dir, tc) = fake_toolchain("spec-settings-present");
    let repo = PathBuf::from("/appdata/m2-repo");
    let settings = PathBuf::from("/appdata/mvnd-settings.xml");

    let spec =
        build_spawn_spec("mvnd package -DskipTests", &repo, Some(&settings), &tc).unwrap();

    assert_eq!(
        spec.args,
        vec![
            "package",
            "-DskipTests",
            "-Dmaven.repo.local=/appdata/m2-repo",
            "-s",
            "/appdata/mvnd-settings.xml",
        ]
    );
}

#[test]
fn settings_xml缺失时不改变参数() {
    let (_dir, tc) = fake_toolchain("spec-settings-absent");
    let repo = PathBuf::from("/appdata/m2-repo");

    let spec = build_spawn_spec("mvnd package", &repo, None, &tc).unwrap();

    assert_eq!(
        spec.args,
        vec!["package", "-Dmaven.repo.local=/appdata/m2-repo"],
        "未设置 settings.xml 时不得追加 -s"
    );
}

#[test]
fn 用户命令已含_s_短参数时不重复追加() {
    let (_dir, tc) = fake_toolchain("spec-settings-short");
    let repo = PathBuf::from("/appdata/m2-repo");
    let settings = PathBuf::from("/appdata/mvnd-settings.xml");

    let spec = build_spawn_spec(
        "mvnd package -s /my/own/settings.xml",
        &repo,
        Some(&settings),
        &tc,
    )
    .unwrap();

    let count = spec.args.iter().filter(|a| a.as_str() == "-s").count();
    assert_eq!(count, 1, "用户已手写 -s 时不得重复追加");
    assert!(spec.args.contains(&"/my/own/settings.xml".to_string()));
    assert!(!spec.args.contains(&"/appdata/mvnd-settings.xml".to_string()));
}

#[test]
fn 用户命令已含_settings_长参数时不重复追加() {
    let (_dir, tc) = fake_toolchain("spec-settings-long");
    let repo = PathBuf::from("/appdata/m2-repo");
    let settings = PathBuf::from("/appdata/mvnd-settings.xml");

    let spec = build_spawn_spec(
        "mvnd package --settings /my/own/settings.xml",
        &repo,
        Some(&settings),
        &tc,
    )
    .unwrap();

    assert!(!spec.args.contains(&"/appdata/mvnd-settings.xml".to_string()));
    assert!(spec.args.contains(&"--settings".to_string()));
}

#[test]
fn 附着写法的_s参数也被识别为已显式给出() {
    let (_dir, tc) = fake_toolchain("spec-settings-attached");
    let repo = PathBuf::from("/appdata/m2-repo");
    let settings = PathBuf::from("/appdata/mvnd-settings.xml");

    let spec = build_spawn_spec("mvnd package -s=/my/settings.xml", &repo, Some(&settings), &tc)
        .unwrap();

    assert!(!spec.args.contains(&"/appdata/mvnd-settings.xml".to_string()));
}
