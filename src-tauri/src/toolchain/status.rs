//! 工具链三态判定：已托管 / 已检测（PATH，仅展示）/ 缺失。
//!
//! ADR-0001：PATH 检测只用于设置页展示，绝不作为执行来源；托管优先。

use serde::Serialize;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::toolchain::catalog::ToolKind;
use crate::toolchain::state::{ManagedEntry, ManagedState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ToolState {
    /// 已托管：本应用数据目录内有登记且可执行文件存在。
    Managed {
        version: String,
        path: String,
    },
    /// 已检测：系统 PATH 上存在，仅展示（ADR-0001）。
    Detected {
        path: String,
    },
    Missing,
}

impl ToolState {
    pub fn is_managed(&self) -> bool {
        matches!(self, ToolState::Managed { .. })
    }
}

/// 某工具在 `tools/<dir>/` 下可执行文件的候选相对路径（不含 jdk 的
/// `Contents/Home` 场景，见 `scan_jdk_home`）。
fn fixed_exec_candidates(kind: ToolKind) -> &'static [&'static str] {
    match kind {
        ToolKind::Git => {
            if cfg!(windows) {
                &["cmd/git.exe", "bin/git.exe"]
            } else {
                &["bin/git"]
            }
        }
        ToolKind::Jdk => {
            if cfg!(windows) {
                &["bin/java.exe"]
            } else {
                &["bin/java"]
            }
        }
        ToolKind::Mvnd => {
            if cfg!(windows) {
                &["bin/mvnd.cmd"]
            } else {
                &["bin/mvnd"]
            }
        }
    }
}

/// 在 `tools/<dir>/` 内定位可执行文件。JDK 发行包在 macOS 上多包一层
/// `Contents/Home`（Zulu zip 实测如此），且顶层目录名随版本变化，故扫描一层。
pub fn find_executable(kind: ToolKind, tools_root: &Path) -> Option<PathBuf> {
    find_executable_in(kind, &tools_root.join(kind.dir_name()))
}

/// 在给定目录内定位工具可执行文件（目录即工具安装根，不含 dir_name 层）。
pub fn find_executable_in(kind: ToolKind, dir: &Path) -> Option<PathBuf> {
    for rel in fixed_exec_candidates(kind) {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if kind == ToolKind::Jdk {
        // 发行包剥掉一层版本目录后，macOS 包有两种常见落位：
        // jdk/Contents/Home/bin/java（Temurin tar.gz）与
        // jdk/<zulu目录>/Contents/Home/bin/java（Zulu zip）。扫一层覆盖两者。
        const NESTED: [&str; 6] = [
            "Contents/Home/bin/java",
            "Contents/Home/bin/java.exe",
            "Home/bin/java",
            "Home/bin/java.exe",
            "bin/java",
            "bin/java.exe",
        ];
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                for rel in NESTED {
                    let candidate = child.join(rel);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

/// 在给定 PATH（`:`/`;` 分隔）上查找工具可执行文件；仅用于状态展示。
fn detect_on_path_var(kind: ToolKind, path_var: &OsStr) -> Option<PathBuf> {
    let name: &OsStr = match kind {
        ToolKind::Git => OsStr::new("git"),
        ToolKind::Jdk => OsStr::new("java"),
        ToolKind::Mvnd => OsStr::new("mvnd"),
    };
    which::which_in(name, Some(path_var), std::env::current_dir().ok()?).ok()
}

/// PATH 上的工具检测（使用进程真实 PATH；macOS Finder 启动时 PATH 很短，
/// 检测不到就如实显示缺失/可通过下载托管）。
pub fn detect_on_path(kind: ToolKind) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    detect_on_path_var(kind, &path_var)
}

/// 综合三态：托管优先于 PATH 检测（ADR-0001）。
pub fn compute_status(kind: ToolKind, tools_root: &Path) -> ToolState {
    compute_status_with(kind, tools_root, detect_on_path)
}

/// PATH 探测可注入的变体：单测不依赖本机真实 PATH。
pub fn compute_status_with(
    kind: ToolKind,
    tools_root: &Path,
    path_lookup: impl Fn(ToolKind) -> Option<PathBuf>,
) -> ToolState {
    // 已托管：状态文件登记 + 可执行文件真实存在（登记被手删目录时降级）。
    let state = ManagedState::load(tools_root);
    if let Some(entry) = state.tools.get(kind.dir_name()) {
        let executable = entry
            .executable_rel
            .parse::<PathBuf>()
            .ok()
            .filter(|rel| tools_root.join(rel).is_file())
            .or_else(|| find_executable(kind, tools_root));
        if let Some(exec) = executable {
            let rel = exec.strip_prefix(tools_root).unwrap_or(&exec);
            return ToolState::Managed {
                version: entry.version.clone(),
                path: rel.to_string_lossy().to_string(),
            };
        }
    }
    if let Some(path) = path_lookup(kind) {
        return ToolState::Detected {
            path: path.to_string_lossy().to_string(),
        };
    }
    ToolState::Missing
}

/// 从 tools_root 汇总全部工具状态（设置页一次拉取）。
pub fn compute_all(tools_root: &Path) -> Vec<(ToolKind, ToolState)> {
    ToolKind::ALL
        .into_iter()
        .map(|kind| (kind, compute_status(kind, tools_root)))
        .collect()
}

/// 下载成功后登记托管条目。
pub fn register_managed(
    kind: ToolKind,
    tools_root: &Path,
    version: &str,
    executable_rel: &Path,
) -> std::io::Result<ManagedEntry> {
    let entry = ManagedEntry {
        version: version.to_string(),
        installed_at: now_rfc3339(),
        executable_rel: executable_rel.to_string_lossy().to_string(),
    };
    let mut state = ManagedState::load(tools_root);
    state
        .tools
        .insert(kind.dir_name().to_string(), entry.clone());
    state.save(tools_root)?;
    Ok(entry)
}

/// 安装完成时间（RFC 3339，仅展示用）。
fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)] // 中文测试名含 PATH 等拉丁缩写
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "packsmith-status-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 缺失_无登记无PATH() {
        let root = temp_root("missing");
        assert_eq!(
            compute_status_with(ToolKind::Mvnd, &root, |_| None),
            ToolState::Missing
        );
    }

    #[test]
    fn 已托管_登记且可执行文件存在() {
        let root = temp_root("managed");
        let exec = root.join("mvnd/bin/mvnd");
        std::fs::create_dir_all(exec.parent().unwrap()).unwrap();
        std::fs::write(&exec, "#!/bin/sh").unwrap();
        register_managed(ToolKind::Mvnd, &root, "1.0.6", Path::new("mvnd/bin/mvnd")).unwrap();

        match compute_status_with(ToolKind::Mvnd, &root, |_| None) {
            ToolState::Managed { version, path } => {
                assert_eq!(version, "1.0.6");
                assert_eq!(path, "mvnd/bin/mvnd");
            }
            other => panic!("应为 Managed，实际 {other:?}"),
        }
    }

    #[test]
    fn 登记在手但目录被删时降级而非误报托管() {
        let root = temp_root("stale");
        register_managed(ToolKind::Mvnd, &root, "1.0.6", Path::new("mvnd/bin/mvnd")).unwrap();
        assert_eq!(
            compute_status_with(ToolKind::Mvnd, &root, |_| None),
            ToolState::Missing
        );
    }

    #[test]
    fn jdk_contents_home嵌套可被找到() {
        let root = temp_root("jdk-home");
        let exec = root.join("jdk/Contents/Home/bin/java");
        std::fs::create_dir_all(exec.parent().unwrap()).unwrap();
        std::fs::write(&exec, "ELF...").unwrap();
        let found = find_executable(ToolKind::Jdk, &root).expect("找到 java");
        assert_eq!(found, exec);
    }

    #[test]
    #[cfg(unix)]
    fn 在指定PATH上检测到工具() {
        let dir = temp_root("pathbin");
        let exec = dir.join("git");
        std::fs::write(&exec, "#!/bin/sh").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();

        let found = detect_on_path_var(ToolKind::Git, dir.as_os_str()).expect("git on PATH");
        assert_eq!(found, exec);
    }

    #[test]
    fn 托管优先于PATH检测() {
        let root = temp_root("priority");
        let exec = root.join("mvnd/bin/mvnd");
        std::fs::create_dir_all(exec.parent().unwrap()).unwrap();
        std::fs::write(&exec, "#!/bin/sh").unwrap();
        register_managed(ToolKind::Mvnd, &root, "1.0.6", Path::new("mvnd/bin/mvnd")).unwrap();

        // PATH 上即使也有同名工具，仍报托管（执行只用托管，ADR-0001）
        let on_path = |_| Some(PathBuf::from("/opt/homebrew/bin/mvnd"));
        assert!(matches!(
            compute_status_with(ToolKind::Mvnd, &root, on_path),
            ToolState::Managed { .. }
        ));
    }
}
