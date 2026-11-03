# 13: Git 账号设置与克隆认证

**What to build:** 设置页新增"Git 账号"卡片（用户名 + 密码/访问令牌，入加密库）。clone 项目与实时拉取分支列表（git2）在远程要求认证时自动使用已配置的凭据（UserPass 回调）；未配置时保持匿名（公开仓库不受影响）；凭据错误时给明确中文错误，引导去设置页检查。凭据永不回传前端明文（沿用 13 号评审后的掩码模式：编辑留空=保持不变，显示"已设置"状态）。

**Blocked by:** None（基于集成 tip）

**Status:** resolved

> 实现：`git_username` / `git_token` 存加密库 `app_meta`（迁移 v1 起即有加密 KV，无需新迁移）；`git_auth` 模块负责 SSH URL 识别、凭据读取（查找失败按匿名，不让 clone 崩溃）与认证回调选择（首次 UserPass，再次被提示报错中止）；clone / 分支拉取 / 打包工作区三条路径均接入；命令层掩码出参（username + hasToken，令牌明文永不过 IPC）；设置页 Git 账号卡片（留空保持不变 + 显式清除 + 已设置/未设置）；i18n `git_auth_failed` / `ssh_not_supported`。真网络端到端验证见 `tests/git_auth_test.rs` 的 `#[ignore]` 用例（需提供真实私有仓库环境变量）。

- [x] 未配置账号：公开仓库 clone/分支拉取照常工作
- [x] 配置账号后：私有（HTTPS）仓库 clone 成功，分支列表可拉取
- [x] 凭据错误：明确中文报错，提示去设置页检查
- [x] 账号存加密库；列表/编辑不回传明文密码（掩码 + 留空保持不变 + 已设置指示）
- [x] SSH 协议 URL（git@github.com:...）不支持时给出明确中文提示（v1.1 仅 HTTPS 认证）
