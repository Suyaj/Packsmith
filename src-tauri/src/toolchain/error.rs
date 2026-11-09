//! 工具链模块错误类型。经 Tauri IPC 序列化为字符串。

use serde::Serialize;
use serde::ser::Serializer;

#[derive(Debug, thiserror::Error)]
pub enum ToolchainError {
    #[error("网络下载失败：{0}")]
    Http(String),
    #[error("当前平台（{os}/{arch}）暂无 {tool} 的官方安装包")]
    NoSourceForPlatform {
        tool: String,
        os: String,
        arch: String,
    },
    #[error("解压安装包失败：{0}")]
    Archive(String),
    #[error("可执行文件权限设置失败：{path}：{detail}")]
    ExecBit { path: String, detail: String },
    #[error("本地文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("{0} 正在下载中，请等待完成")]
    Busy(String),
    #[error("本地存储操作失败：{0}")]
    Store(String),
}

impl From<crate::store::StoreError> for ToolchainError {
    fn from(e: crate::store::StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

impl Serialize for ToolchainError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
