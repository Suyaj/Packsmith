# 02: 加密持久化底座

**What to build:** 应用启动时自动初始化单库 SQLCipher 加密数据库（rusqlite），加密密钥首次启动自动生成并存入系统钥匙串（macOS Keychain / Windows Credential Manager），全程无用户输入。提供后续所有票共用的数据访问入口与迁移机制（ADR-0002）。

**Blocked by:** 01

**Status:** resolved

- [ ] 首次启动自动建库 + 密钥入钥匙串，用户无感知
- [ ] 写入测试数据 → 重启应用仍在
- [ ] 把数据库文件拷到别处直接打开无法读出内容
- [ ] 后续票可复用统一的数据访问/迁移入口
