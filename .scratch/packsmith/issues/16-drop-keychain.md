# 16: 取消钥匙串访问——SQLCipher 密钥改存密钥文件

**What to build:** 移除系统钥匙串依赖（用户反馈：钥匙串弹窗骚扰）。SQLCipher 密钥改为存应用数据目录下的密钥文件 `<appdata>/db.key`（0600 权限，仅当前用户可读）。数据库依旧整库加密（ADR 威胁模型不变：只拷走 db 文件仍读不出凭据；拷走整个 appdata 目录则可读——这是用户明确接受的取舍）。

**迁移路径（关键，做错会丢用户数据）**：启动时读取顺序——①`db.key` 存在 → 直接用；②不存在 → 尝试从系统钥匙串读旧密钥 → 成功则写入 `db.key`（0600）并**从钥匙串删除旧条目**，用旧密钥打开既有数据库（用户数据无损）；③钥匙串也没有 → 生成新密钥写 `db.key`（全新安装）。

**文档**：新增 `docs/adr/0003-key-file-instead-of-keychain.md`（记录取舍：零弹窗 ↔ 拷贝整个 appdata 可解密），并将 `docs/adr/0002-sqlcipher-encrypted-store.md` 状态改为 `superseded by ADR-0003`（加密库本身的部分仍然有效，在 0003 中说明）。

**Blocked by:** None（基于集成 tip）

**Status:** ready-for-agent

- [ ] 全新安装：生成密钥文件（0600），启动零钥匙串访问
- [ ] 老用户升级：钥匙串旧密钥迁移到文件，既有数据库可读，钥匙串条目被清理
- [ ] 运行期任何代码路径不再触碰 keyring crate（可从 Cargo.toml 移除依赖）
- [ ] `db.key` 权限 0600；db 文件拷走仍不可读（既有加密测试仍绿）
- [ ] ADR-0003 就位、ADR-0002 标记 superseded
