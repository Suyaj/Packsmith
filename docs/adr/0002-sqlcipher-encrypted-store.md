---
status: superseded by ADR-0003
---

# 单库 SQLCipher 加密存储，密钥托管于系统钥匙串

Packsmith 的全部结构化数据（项目、服务器、部署目标、构建历史）与敏感凭据（git token、SSH 密码、passphrase）集中存放在一个 SQLCipher 整库加密的 SQLite 数据库中（rusqlite），加密密钥为首次启动自动生成的随机密钥，存放在操作系统钥匙串（macOS Keychain / Windows Credential Manager），用户全程无感。被否决的方案：tauri-plugin-store JSON + 敏感项单独进钥匙串（配置分散、无法对整库加密）；用户主密码派生密钥（安全边界更强，但每次启动都有输入成本）。选择的前提是威胁模型为"防数据库文件被拷走即读出凭据"，而非"防本机用户"。

> **已被 ADR-0003 部分取代（ticket 16）**：加密单库的核心设计仍然有效；密钥存储由系统钥匙串改为应用数据目录密钥文件 `<appdata>/db.key`（0600），原因与迁移路径见 ADR-0003。
