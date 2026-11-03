//! 托管工具的官方下载源目录（catalog）。
//!
//! 每个工具在每个 (OS, arch) 组合下至多一个官方安装包来源；没有官方
//! 预编译包的组合（如 macOS/Linux 上的 git）返回 `None`，UI 据此不提供
//! 下载按钮（见 ADR-0001 与票 03：各工具托管一个版本）。
//!
//! 来源说明：
//! - mvnd：apache/maven-mvnd GitHub Releases 官方资产（zip）
//! - JDK 8：macOS aarch64 用 Azul Zulu 8（Temurin 8 无 mac aarch64 构建）；
//!   其余平台用 Adoptium Temurin 8 稳定 API 链接（跟随最新 8u）
//! - git：仅 Windows 有官方 zip（Git for Windows 的 MinGit）；macOS/Linux
//!   官方仅分发源码或系统包，v1 不提供托管下载

use serde::{Deserialize, Serialize};

pub const GIT_DIR: &str = "git";
pub const JDK_DIR: &str = "jdk";
pub const MVND_DIR: &str = "mvnd";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Git,
    Jdk,
    Mvnd,
}

impl ToolKind {
    pub const ALL: [ToolKind; 3] = [ToolKind::Git, ToolKind::Jdk, ToolKind::Mvnd];

    pub fn dir_name(self) -> &'static str {
        match self {
            ToolKind::Git => GIT_DIR,
            ToolKind::Jdk => JDK_DIR,
            ToolKind::Mvnd => MVND_DIR,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

/// 一个工具在特定平台上的官方安装包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSource {
    pub url: String,
    pub format: ArchiveFormat,
    /// 目录中登记的版本（对滚动链接取主版本描述）。
    pub version: &'static str,
    /// 发行包是否包一层顶层版本目录（true 时解压剥离）。
    pub strip_top: bool,
}

const ADOPTIOM_BASE: &str = "https://api.adoptium.net/v3/binary/latest/8/ga";

/// 纯函数查表：给定平台返回官方安装包；无官方包返回 None。
pub fn source_for(kind: ToolKind, os: &str, arch: &str) -> Option<ToolSource> {
    match (kind, os, arch) {
        // --- mvnd：全平台 zip ---
        (ToolKind::Mvnd, "macos", "aarch64") => Some(mvnd("maven-mvnd-1.0.6-darwin-aarch64.zip")),
        (ToolKind::Mvnd, "macos", "x86_64") => Some(mvnd("maven-mvnd-1.0.6-darwin-amd64.zip")),
        (ToolKind::Mvnd, "linux", "aarch64") => Some(mvnd("maven-mvnd-1.0.6-linux-aarch64.zip")),
        (ToolKind::Mvnd, "linux", "x86_64") => Some(mvnd("maven-mvnd-1.0.6-linux-amd64.zip")),
        (ToolKind::Mvnd, "windows", "x86_64") => Some(mvnd("maven-mvnd-1.0.6-windows-amd64.zip")),

        // --- JDK 8 ---
        (ToolKind::Jdk, "macos", "aarch64") => Some(ToolSource {
            url: "https://cdn.azul.com/zulu/bin/zulu8.98.0.107-ca-jdk8.0.512-macosx_aarch64.zip"
                .to_string(),
            format: ArchiveFormat::Zip,
            version: "8u512",
            strip_top: true,
        }),
        (ToolKind::Jdk, "macos", "x86_64") => Some(ToolSource {
            url: "{BASE}/mac/x64/jdk/hotspot/normal/eclipse".to_string(),
            format: ArchiveFormat::TarGz,
            version: "8 (Temurin latest)",
            strip_top: true,
        }),
        (ToolKind::Jdk, "linux", "aarch64") => Some(ToolSource {
            url: "{BASE}/linux/aarch64/jdk/hotspot/normal/eclipse".to_string(),
            format: ArchiveFormat::TarGz,
            version: "8 (Temurin latest)",
            strip_top: true,
        }),
        (ToolKind::Jdk, "linux", "x86_64") => Some(ToolSource {
            url: "{BASE}/linux/x64/jdk/hotspot/normal/eclipse".to_string(),
            format: ArchiveFormat::TarGz,
            version: "8 (Temurin latest)",
            strip_top: true,
        }),
        (ToolKind::Jdk, "windows", "x86_64") => Some(ToolSource {
            url: "{BASE}/windows/x64/jdk/hotspot/normal/eclipse".to_string(),
            format: ArchiveFormat::Zip,
            version: "8 (Temurin latest)",
            strip_top: true,
        }),

        // --- git：仅 Windows 有官方 zip（MinGit）---
        (ToolKind::Git, "windows", "x86_64") => Some(ToolSource {
            url: "https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.1/MinGit-2.47.1-64-bit.zip".to_string(),
            format: ArchiveFormat::Zip,
            version: "2.47.1",
            strip_top: false, // MinGit zip 无顶层目录，条目直接落位
        }),

        // macOS / Linux 的 git 无官方预编译包；其余组合当前不支持。
        _ => None,
    }
}

/// 运行时入口：当前平台的官方安装包，URL 已展开。
pub fn current_source(kind: ToolKind) -> Option<ToolSource> {
    source_for(kind, std::env::consts::OS, std::env::consts::ARCH).map(expand)
}

/// 展开模板 URL 中的 Adoptium base（避免常量拼接噪音）。
fn expand(mut src: ToolSource) -> ToolSource {
    if src.url.contains("{BASE}") {
        src.url = src.url.replace("{BASE}", ADOPTIOM_BASE);
    }
    src
}

fn mvnd(file: &'static str) -> ToolSource {
    ToolSource {
        url: format!("https://github.com/apache/maven-mvnd/releases/download/1.0.6/{file}"),
        format: ArchiveFormat::Zip,
        version: "1.0.6",
        strip_top: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mvnd_全平台均有官方zip() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
            ("windows", "x86_64"),
        ] {
            let src = source_for(ToolKind::Mvnd, os, arch).expect("mvnd source");
            assert_eq!(src.format, ArchiveFormat::Zip);
            assert!(src.url.starts_with("https://github.com/apache/maven-mvnd/releases/"));
            assert!(src.url.ends_with(".zip"));
        }
    }

    #[test]
    fn mvnd_来源匹配当前平台() {
        let src = current_source(ToolKind::Mvnd).unwrap();
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let expected_tag = match (os, arch) {
            ("macos", "aarch64") => "darwin-aarch64",
            ("macos", "x86_64") => "darwin-amd64",
            ("linux", "aarch64") => "linux-aarch64",
            ("linux", "x86_64") => "linux-amd64",
            ("windows", "x86_64") => "windows-amd64",
            other => panic!("未覆盖的平台 {other:?}"),
        };
        assert!(src.url.contains(expected_tag), "{} 应含 {}", src.url, expected_tag);
    }

    #[test]
    fn jdk8_本机有源且zip格式可解包() {
        let src = source_for(ToolKind::Jdk, "macos", "aarch64").unwrap();
        assert_eq!(src.format, ArchiveFormat::Zip);
        assert!(src.url.starts_with("https://cdn.azul.com/zulu/bin/"));
    }

    #[test]
    fn jdk8_其余平台走temurin稳定链接() {
        let src = expand(source_for(ToolKind::Jdk, "linux", "x86_64").unwrap());
        assert_eq!(
            src.url,
            "https://api.adoptium.net/v3/binary/latest/8/ga/linux/x64/jdk/hotspot/normal/eclipse"
        );
    }

    #[test]
    fn git_仅windows有官方包() {
        assert!(source_for(ToolKind::Git, "windows", "x86_64").is_some());
        assert!(source_for(ToolKind::Git, "macos", "aarch64").is_none());
        assert!(source_for(ToolKind::Git, "linux", "x86_64").is_none());
    }

    #[test]
    fn 目录名与工具一一对应() {
        assert_eq!(ToolKind::Git.dir_name(), "git");
        assert_eq!(ToolKind::Jdk.dir_name(), "jdk");
        assert_eq!(ToolKind::Mvnd.dir_name(), "mvnd");
    }
}
