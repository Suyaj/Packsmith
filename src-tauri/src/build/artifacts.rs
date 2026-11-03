//! 构件扫描与入库（ticket 08）。
//!
//! 构建成功后，在临时工作区内扫描 Maven 输出：所有名为 `target` 的目录
//! （多模块工程的每个模块各有一个）下的 `*.jar` / `*.war` 文件即为构件。
//! 扫描结果带工作区内相对路径，供入库复制与前端选择展示。
//!
//! 构件库布局：`<appdata>/artifacts/<项目名>/<构建时间>/`，构建时间目录名由
//! [`timestamp_dir_name`] 生成（UTC `YYYYMMDD-HHMMSS`；同秒冲突时追加纳秒
//! 后缀保证唯一）。入库复制在临时工作区删除**之前**进行（构件必须先抢救
//! 出临时工作区，GLOSSARY：构件）。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// 一个扫描到的构件：工作区内相对路径 + 文件名 + 字节大小。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScannedArtifact {
    /// 相对临时工作区根的路径（`/` 分隔，跨平台稳定，前端可直接展示）。
    pub relative_path: String,
    pub file_name: String,
    pub size: u64,
}

/// 扫描 `workspace` 内所有 `target` 目录（递归，多模块感知）下的
/// jar/war 构件。目录不存在或无构件时返回空。结果按相对路径排序，稳定。
pub fn scan_workspace_artifacts(workspace: &Path) -> Vec<ScannedArtifact> {
    let mut out = Vec::new();
    if walk_targets(workspace, workspace, &mut out).is_err() {
        // 目录不可读/不存在：等价于没有构件（调用方按“无构件”处理）。
        return Vec::new();
    }
    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    out
}

fn walk_targets(root: &Path, dir: &Path, out: &mut Vec<ScannedArtifact>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if entry.file_name() == "target" {
                collect_artifacts(root, &path, out)?;
            } else {
                walk_targets(root, &path, out)?;
            }
        }
    }
    Ok(())
}

/// 收集单个 `target` 目录下的构件（递归：repackage 等输出可能在子目录）。
fn collect_artifacts(
    root: &Path,
    target_dir: &Path,
    out: &mut Vec<ScannedArtifact>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(target_dir)?.collect::<Result<Vec<_>, _>>()? {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_artifacts(root, &path, out)?;
        } else if is_artifact_name(&entry.file_name().to_string_lossy()) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let relative_path = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(ScannedArtifact {
                file_name: entry.file_name().to_string_lossy().into_owned(),
                relative_path,
                size,
            });
        }
    }
    Ok(())
}

fn is_artifact_name(name: &str) -> bool {
    name.ends_with(".jar") || name.ends_with(".war")
}

// ---------------------------------------------------------------------------
// 构件库目录与复制
// ---------------------------------------------------------------------------

/// 构件库根目录：`<appdata>/artifacts`。
pub fn library_root(appdata: &Path) -> PathBuf {
    appdata.join("artifacts")
}

/// 构建时间目录名（UTC `YYYYMMDD-HHMMSS`，spec: `<appdata>/artifacts/<项目>/
/// <构建时间>/`）。
pub fn timestamp_dir_name(at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hour, min, sec) = (rem / 3600, rem % 3600 / 60, rem % 60);
    format!("{y:04}{m:02}{d:02}-{hour:02}{min:02}{sec:02}")
}

/// Howard Hinnant 的 civil_from_days：1970-01-01 起的天数 → (年, 月, 日)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 项目本次构建的构件库目录：`<appdata>/artifacts/<项目名>/<构建时间>/`。
///
/// 同一秒内同名目录已存在时追加 `-<纳秒>` 后缀，保证多次构建互不覆盖。
pub fn artifacts_dir_for(appdata: &Path, project_name: &str, at: SystemTime) -> PathBuf {
    let project_dir = library_root(appdata).join(project_name);
    let base = timestamp_dir_name(at);
    let mut dir = project_dir.join(&base);
    if dir.exists() {
        let nanos = at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir = project_dir.join(format!("{base}-{nanos}"));
    }
    dir
}

/// 把扫描到的构件复制进 `dest` 目录（平铺，保留原文件名；上传部署按原文件
/// 名覆盖，GLOSSARY/部署）。返回复制后的库内路径列表，顺序与输入一致。
pub fn copy_artifacts_into_library(
    workspace: &Path,
    artifacts: &[ScannedArtifact],
    dest: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dest)?;
    let mut copied = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let from = workspace.join(&artifact.relative_path);
        let to = dest.join(&artifact.file_name);
        std::fs::copy(&from, &to)?;
        copied.push(to);
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(non_snake_case)]
    fn 时间目录名为UTC的YYYYMMDD_HHMMSS() {
        // 2026-01-01T00:00:00Z = 1_767_225_600。
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_767_225_600);
        assert_eq!(timestamp_dir_name(t), "20260101-000000");
        let noon = t + std::time::Duration::from_secs(15 * 3600 + 30 * 60);
        assert_eq!(timestamp_dir_name(noon), "20260101-153000");
    }
}
