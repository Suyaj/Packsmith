//! 托管工具链状态的持久化：`<appdata>/toolchain-state.json`。
//!
//! 本票不依赖票 02 的加密库，先落 JSON 文件（ticket 03 决策）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 单个托管工具的登记信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedEntry {
    /// 安装的版本号，取自安装包目录名（如 "1.0.6"、"8u512"）。
    pub version: String,
    /// 安装完成时间（RFC 3339）。
    pub installed_at: String,
    /// 相对 `tools/` 根的可执行文件路径（如 "jdk/Contents/Home/bin/java"）。
    pub executable_rel: String,
}

/// 持久化的托管工具状态，键为工具目录名（git / jdk / mvnd）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ManagedState {
    #[serde(default, flatten)]
    pub tools: BTreeMap<String, ManagedEntry>,
}

impl ManagedState {
    pub fn state_path(tools_root: &Path) -> PathBuf {
        tools_root.join("toolchain-state.json")
    }

    /// 读取状态文件；文件不存在或损坏时返回空状态（不抛错，视为未托管）。
    pub fn load(tools_root: &Path) -> ManagedState {
        let path = Self::state_path(tools_root);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => ManagedState::default(),
        }
    }

    /// 原子写入（先写临时文件再改名），避免下载完成后崩溃留下半截状态。
    pub fn save(&self, tools_root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(tools_root)?;
        let path = Self::state_path(tools_root);
        let tmp = tools_root.join(format!(
            ".{}.tmp",
            path.file_name().unwrap().to_string_lossy()
        ));
        let json = serde_json::to_string_pretty(self).expect("state serializes");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "packsmith-state-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn 登记后保存再读出一致() {
        let root = temp_root("roundtrip");
        let mut state = ManagedState::default();
        state.tools.insert(
            "mvnd".into(),
            ManagedEntry {
                version: "1.0.6".into(),
                installed_at: "2026-11-02T10:00:00Z".into(),
                executable_rel: "mvnd/bin/mvnd".into(),
            },
        );
        state.save(&root).expect("save");

        let loaded = ManagedState::load(&root);
        assert_eq!(loaded, state);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 状态文件缺失时视为未托管() {
        let root = temp_root("missing");
        let state = ManagedState::load(&root);
        assert!(state.tools.is_empty());
    }

    #[test]
    fn 状态文件损坏时视为未托管而非报错() {
        let root = temp_root("corrupt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(ManagedState::state_path(&root), "{ not json").unwrap();
        let state = ManagedState::load(&root);
        assert!(state.tools.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
