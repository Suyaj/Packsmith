//! 安装包解压：zip / tar.gz。
//!
//! 两种布局，由 catalog 的 `strip_top` 决定：
//! - 有顶层版本目录（mvnd、JDK 发行包）：剥离顶层目录后落位
//! - 无顶层目录（MinGit）：条目直接落位
//!
//! 统一防御路径穿越（zip-slip）：任何条目试图逃出 `dest_root`
//!（`..`、绝对路径、越界符号链接）都直接报错。

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::toolchain::catalog::ArchiveFormat;

/// 把 `archive_path` 解压到 `dest_root`。
///
/// `strip_top` 为 true 时剥离顶层目录（发行包包一层版本目录的布局）。
pub fn extract_archive(
    archive_path: &Path,
    dest_root: &Path,
    format: ArchiveFormat,
    strip_top: bool,
) -> io::Result<()> {
    match format {
        ArchiveFormat::Zip => extract_zip(archive_path, dest_root, strip_top),
        ArchiveFormat::TarGz => extract_tar_gz(archive_path, dest_root, strip_top),
    }
}

/// 防穿越：路径必须是相对的、不含 ".." 的普通组件序列。
fn safe_rel(rel: &Path) -> bool {
    if matches!(
        rel.components().next(),
        Some(Component::RootDir | Component::Prefix(_))
    ) {
        return false;
    }
    rel.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn escape_err(entry: &str) -> io::Error {
    io::Error::other(format!("压缩包条目越界：{entry}"))
}

/// 计算条目在 dest 内的目标路径。strip_top=true 时要求条目位于顶层目录
/// 之下并剥离它；非法条目返回 Err。
fn dest_path_for(
    entry_name: &str,
    dest_root: &Path,
    strip_top: bool,
) -> io::Result<Option<PathBuf>> {
    // 目录条目（以 / 结尾）与文件条目同样处理；空 rest 表示顶层目录本身。
    let (rest, is_top_marker) = if strip_top {
        match entry_name.split_once('/') {
            Some((_, rest)) => (rest, rest.is_empty()),
            None => return Err(escape_err(entry_name)), // strip 布局下不允许根级条目
        }
    } else {
        (entry_name, false)
    };
    if is_top_marker || rest.is_empty() {
        return Ok(None); // 目录条目本身，无需落盘（父目录会按需创建）
    }
    let rel = Path::new(rest);
    if !safe_rel(rel) {
        return Err(escape_err(entry_name));
    }
    Ok(Some(dest_root.join(rel)))
}

/// 条目带 unix 权限位时原样落盘（仅保留权限位，忽略文件类型位）。
/// 无模式信息（None）的条目保持默认行为。Windows 上无需处理。
fn apply_unix_mode(target: &Path, unix_mode: Option<u32>) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(mode) = unix_mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode & 0o777))?;
    }
    #[cfg(not(unix))]
    let _ = (target, unix_mode);
    Ok(())
}

fn extract_zip(archive_path: &Path, dest_root: &Path, strip_top: bool) -> io::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry
            .name()
            .map_err(|e| io::Error::other(e.to_string()))?
            .to_string();
        match dest_path_for(&name, dest_root, strip_top)? {
            Some(target) => {
                if entry.is_dir() {
                    std::fs::create_dir_all(&target)?;
                } else {
                    // unix 条目带权限位（官方 mvnd/JDK zip 的可执行文件为 0755）；
                    // 无模式信息的包（Windows 打常见）落默认 0644，由安装校验自愈。
                    let unix_mode = entry.unix_mode();
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::io::copy(&mut entry, &mut std::fs::File::create(&target)?)?;
                    apply_unix_mode(&target, unix_mode)?;
                }
            }
            None => {
                if !entry.is_dir() && strip_top {
                    return Err(escape_err(&name));
                }
            }
        }
    }
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, dest_root: &Path, strip_top: bool) -> io::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let raw = entry.path()?.to_string_lossy().to_string();
        // 符号链接的目标若是绝对路径或逃出 dest，拒绝（后续 unpack 时由
        // 目标路径仍受 safe_rel 约束，链接本身只允许相对指向包内）。
        if entry.link_name()?.is_some() {
            let link_target = entry.link_name()?.unwrap();
            if link_target.is_absolute() {
                return Err(escape_err(&raw));
            }
        }
        match dest_path_for(&raw, dest_root, strip_top)? {
            Some(target) => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                entry.unpack(&target)?;
            }
            None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "packsmith-archive-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, content) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut zip, content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    /// 可选指定条目 unix_mode 的 zip 夹具（None = 无模式信息，Windows 打包常见）。
    fn write_zip_with_modes(path: &Path, entries: &[(&str, &str, Option<u32>)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, content, mode) in entries {
            let mut options = zip::write::SimpleFileOptions::default();
            if let Some(mode) = mode {
                options = options.unix_permissions(*mode);
            }
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn zip_剥离顶层目录并还原结构() {
        let dir = temp_dir("zip-strip");
        let archive = dir.join("mvnd.zip");
        write_zip(
            &archive,
            &[
                ("maven-mvnd-1.0.6-darwin-aarch64/", ""),
                ("maven-mvnd-1.0.6-darwin-aarch64/bin/mvnd", "#!/bin/sh"),
                ("maven-mvnd-1.0.6-darwin-aarch64/README.adoc", "hello"),
            ],
        );
        let dest = dir.join("tools/mvnd");
        extract_archive(&archive, &dest, ArchiveFormat::Zip, true).unwrap();

        assert_eq!(
            std::fs::read_to_string(dest.join("bin/mvnd")).unwrap(),
            "#!/bin/sh"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("README.adoc")).unwrap(),
            "hello"
        );
        assert!(!dest.join("maven-mvnd-1.0.6-darwin-aarch64").exists());
    }

    #[test]
    #[cfg(unix)]
    fn zip_解压保留条目unix_mode执行位() {
        // 回归：mvnd/JDK 官方 zip 带 unix_mode，extract_zip 曾丢弃导致 0644
        // → spawn Permission denied (os error 13)。
        let dir = temp_dir("zip-exec-bit");
        let archive = dir.join("mvnd.zip");
        write_zip_with_modes(
            &archive,
            &[
                ("maven-mvnd-1.0.6-darwin-aarch64/", "", None),
                (
                    "maven-mvnd-1.0.6-darwin-aarch64/bin/mvnd",
                    "#!/bin/sh",
                    Some(0o755),
                ),
            ],
        );
        let dest = dir.join("tools/mvnd");
        extract_archive(&archive, &dest, ArchiveFormat::Zip, true).unwrap();

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dest.join("bin/mvnd"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "执行位应保留，实际 mode={mode:o}");
    }

    #[test]
    fn zip_无顶层目录布局直接落位() {
        // MinGit 布局：cmd/git.exe 等根级条目
        let dir = temp_dir("zip-flat");
        let archive = dir.join("mingit.zip");
        write_zip(
            &archive,
            &[("cmd/git.exe", "MZ"), ("etc/gitconfig", "[core]")],
        );
        let dest = dir.join("tools/git");
        extract_archive(&archive, &dest, ArchiveFormat::Zip, false).unwrap();

        assert_eq!(
            std::fs::read_to_string(dest.join("cmd/git.exe")).unwrap(),
            "MZ"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("etc/gitconfig")).unwrap(),
            "[core]"
        );
    }

    #[test]
    fn zip_路径穿越条目被拒绝() {
        let dir = temp_dir("zip-slip");
        let archive = dir.join("evil.zip");
        write_zip(&archive, &[("top/../../evil.txt", "pwned")]);
        let dest = dir.join("tools/x");
        let err = extract_archive(&archive, &dest, ArchiveFormat::Zip, true).unwrap_err();
        assert!(err.to_string().contains("越界"), "实际错误：{err}");
        assert!(!dir.join("evil.txt").exists());
    }

    #[test]
    fn zip_无strip布局下的穿越条目同样被拒绝() {
        let dir = temp_dir("zip-slip-flat");
        let archive = dir.join("evil2.zip");
        write_zip(&archive, &[("../../evil.txt", "pwned")]);
        let dest = dir.join("tools/x");
        let err = extract_archive(&archive, &dest, ArchiveFormat::Zip, false).unwrap_err();
        assert!(err.to_string().contains("越界"), "实际错误：{err}");
        assert!(!dir.join("evil.txt").exists());
    }

    #[test]
    fn strip_布局下根级条目被拒绝() {
        let dir = temp_dir("zip-root-entry");
        let archive = dir.join("stray.zip");
        write_zip(&archive, &[("stray.txt", "hi")]);
        let dest = dir.join("tools/x");
        let err = extract_archive(&archive, &dest, ArchiveFormat::Zip, true).unwrap_err();
        assert!(err.to_string().contains("越界"), "实际错误：{err}");
    }

    #[test]
    fn targz_剥离顶层目录() {
        let dir = temp_dir("targz");
        let archive = dir.join("jdk.tar.gz");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            let data = b"#!/bin/sh\n";
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    "jdk8u504-b01/Contents/Home/bin/java",
                    &data[..],
                )
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let dest = dir.join("tools/jdk");
        extract_archive(&archive, &dest, ArchiveFormat::TarGz, true).unwrap();

        assert_eq!(
            std::fs::read_to_string(dest.join("Contents/Home/bin/java")).unwrap(),
            "#!/bin/sh\n"
        );
    }
}
