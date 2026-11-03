//! 构建执行（ticket 07）：托管工具链进程的启动规格与执行。
//!
//! ADR-0001（三重隔离）在此落地的两处：
//! - **执行只用托管工具链**：[`resolve_toolchain`] 只在 `<appdata>/tools/`
//!   内定位 mvnd 与 JDK；构建命令的第一个词必须是 `mvnd` / `mvn`，替换为
//!   托管 mvnd 可执行文件后才启动。
//! - **依赖仓库隔离**：[`build_spawn_spec`] 统一追加
//!   `-Dmaven.repo.local=<appdata>/m2-repo`（用户命令已显式给出时不重复）。
//!
//! 另外，macOS 图形启动（Finder）的 PATH 极短（研究笔记
//! tauri2-commands-state.md）：spawn 时显式注入 JAVA_HOME，并把托管 JDK 的
//! `bin` 与 mvnd 的 `bin` 前置到 PATH。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::toolchain::{ToolKind, find_executable};

use super::BuildError;

/// 托管工具链中构建所需的两项：mvnd 可执行文件与 JAVA_HOME。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainPaths {
    /// 托管 mvnd 可执行文件（绝对路径）。
    pub mvnd: PathBuf,
    /// 托管 JDK 的 home（`bin/java` 的上两级）。
    pub java_home: PathBuf,
}

/// 一次构建进程的启动规格：程序、参数与显式注入的环境变量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// 追加（而非替换）到子进程环境的环境变量。
    pub env: Vec<(String, String)>,
}

/// 在托管工具链根目录内解析 mvnd 与 JDK 8（ADR-0001：只看 `tools/`，
/// 绝不回退 PATH）。
pub fn resolve_toolchain(tools_root: &Path) -> Result<ToolchainPaths, BuildError> {
    let mvnd =
        find_executable(ToolKind::Mvnd, tools_root).ok_or(BuildError::ToolchainMissing("mvnd"))?;
    let java =
        find_executable(ToolKind::Jdk, tools_root).ok_or(BuildError::ToolchainMissing("jdk"))?;
    // java 位于 <home>/bin/java（含 macOS Contents/Home 布局），home 即其上级。
    let java_home = java
        .parent()
        .and_then(Path::parent)
        .ok_or(BuildError::ToolchainMissing("jdk"))?
        .to_path_buf();
    Ok(ToolchainPaths { mvnd, java_home })
}

/// 把命令行拆成词（支持成对的单/双引号包裹含空格的参数）。空命令报错。
pub fn split_command(line: &str) -> Result<Vec<String>, BuildError> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                has_token = true;
            }
            None if ch.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            None => {
                cur.push(ch);
                has_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err(BuildError::InvalidBuildCommand(line.to_string()));
    }
    if has_token {
        tokens.push(cur);
    }
    if tokens.is_empty() {
        return Err(BuildError::InvalidBuildCommand(line.to_string()));
    }
    Ok(tokens)
}

/// 保存到项目上的构建命令的校验：能分词，且第一个词是 mvnd/mvn（v1 仅
/// Maven 项目；执行时会替换为托管 mvnd）。
pub fn validate_build_command(line: &str) -> Result<(), BuildError> {
    let tokens = split_command(line)?;
    match tokens[0].as_str() {
        "mvnd" | "mvn" => Ok(()),
        other => Err(BuildError::UnsupportedBuildCommand(other.to_string())),
    }
}

const REPO_LOCAL_FLAG: &str = "-Dmaven.repo.local";

/// 由构建命令、依赖仓库目录与托管工具链构造启动规格。
///
/// - 第一个词 `mvnd` / `mvn` → 替换为托管 mvnd 可执行文件；
/// - 其余词原样作为参数；
/// - 未显式给出 `-Dmaven.repo.local=…` 时统一追加（ADR-0001）；
/// - 显式注入 JAVA_HOME 与前置 PATH（Finder PATH 坑）。
pub fn build_spawn_spec(
    line: &str,
    repo_local: &Path,
    tc: &ToolchainPaths,
) -> Result<SpawnSpec, BuildError> {
    let tokens = split_command(line)?;
    let program = match tokens[0].as_str() {
        "mvnd" | "mvn" => tc.mvnd.clone(),
        other => return Err(BuildError::UnsupportedBuildCommand(other.to_string())),
    };

    let mut args: Vec<String> = tokens[1..].to_vec();
    if !args.iter().any(|a| a.starts_with(REPO_LOCAL_FLAG)) {
        args.push(format!("{}={}", REPO_LOCAL_FLAG, repo_local.display()));
    }

    let jdk_bin = tc.java_home.join("bin").to_string_lossy().into_owned();
    let mvnd_bin = tc
        .mvnd
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var_os("PATH").map(|p| p.to_string_lossy().into_owned());
    let path = join_path_list(&[jdk_bin, mvnd_bin], existing.as_deref());

    Ok(SpawnSpec {
        program,
        args,
        env: vec![
            (
                "JAVA_HOME".into(),
                tc.java_home.to_string_lossy().into_owned(),
            ),
            ("PATH".into(), path),
        ],
    })
}

/// 拼接平台相关的 PATH 列表（前置项 + 可选的现有值），用于注入子进程。
fn join_path_list(prepend: &[String], existing: Option<&str>) -> String {
    let mut parts: Vec<PathBuf> = prepend.iter().map(PathBuf::from).collect();
    if let Some(rest) = existing {
        parts.extend(std::env::split_paths(rest));
    }
    std::env::join_paths(parts)
        .unwrap_or_else(|_| std::ffi::OsString::from(prepend.join(":")))
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// 进程执行
// ---------------------------------------------------------------------------

/// 构建结果：进程退出码与成败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
}

/// 取消/终止开关：取消命令把它置位，[`execute_build`] 的轮询循环随即对
/// 子进程 `start_kill()`（研究笔记：仅 drop future 不会杀死子进程）。
pub type KillSwitch = Arc<std::sync::atomic::AtomicBool>;

pub fn new_kill_switch() -> KillSwitch {
    Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// 在 `project_dir` 中按 [`SpawnSpec`] 执行构建，逐行推送日志。
///
/// 取消语义：`cancel` 标志置位后（[`cancel`](super::commands) 命令或测试），
/// 本函数轮询到即对子进程 `start_kill()`，并在收尾后返回
/// [`BuildError::Cancelled`]。`on_log` 在独立读取任务中被调用，需自行保证
/// 线程安全。
pub async fn execute_build(
    spec: SpawnSpec,
    project_dir: &Path,
    on_log: Arc<dyn Fn(String) + Send + Sync>,
    cancel: KillSwitch,
) -> Result<ExecOutcome, BuildError> {
    // Windows 上 .cmd/.bat 不能直接 spawn，经由 cmd /c。
    let is_script = matches!(
        spec.program.extension().and_then(|e| e.to_str()),
        Some("cmd") | Some("bat")
    );
    let (program, args): (PathBuf, Vec<String>) = if is_script {
        let mut a = vec![
            "/c".to_string(),
            spec.program.to_string_lossy().into_owned(),
        ];
        a.extend(spec.args.iter().cloned());
        (PathBuf::from("cmd"), a)
    } else {
        (spec.program.clone(), spec.args.clone())
    };

    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args)
        .current_dir(project_dir)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| BuildError::Spawn(e.to_string()))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    fn read_pipe(
        pipe: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        on_log: Arc<dyn Fn(String) + Send + Sync>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_log(line);
            }
        })
    }
    let readers = vec![read_pipe(stdout, on_log.clone()), read_pipe(stderr, on_log)];

    // 轮询等待：每 100ms 检查一次取消标志，置位即杀子进程；进程自行退出
    // 也会从 wait 分支返回。
    let mut killed = false;
    let exit_status = loop {
        tokio::select! {
            status = child.wait() => break status,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                if !killed && cancel.load(Ordering::Relaxed) {
                    let _ = child.start_kill();
                    killed = true;
                }
            }
        }
    };

    // 等读取任务收尾（进程退出后管道随即 EOF），保证日志完整后再返回。
    for reader in readers {
        let _ = reader.await;
    }

    let exit_status = exit_status.map_err(|e| BuildError::Spawn(e.to_string()))?;
    if killed || cancel.load(Ordering::Relaxed) {
        return Err(BuildError::Cancelled);
    }
    let exit_code = exit_status.code();
    Ok(ExecOutcome {
        success: exit_status.success(),
        exit_code,
    })
}

#[cfg(test)]
mod tests;
