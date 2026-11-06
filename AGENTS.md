# Packsmith

## Project overview

Packsmith 是一个打包的桌面应用（当前支持 Java 打包，后续支持其他项目类型）。

- **基础框架**: Tauri（Rust 后端 + WebView 前端）
- **UI**: shadcn-ui
- **首页**: 包含其他页面的相关信息（入口/总览）
- **项目管理**: 支持多种项目类型，目前只实现 Java 相关——通过 git 获取项目，使用 mvnd 进行打包
- **服务器管理**: 用于打包文件的上传与命令行启动相关操作，需要 SSH 管理
- **设置**: 当前项目的版本信息，git、mvnd、java 等第三方工具依赖的检查和下载

## Working conventions

- **Git worktrees**: 所有实现用 worktree 一律放在 `/Users/wyy/RustRoverProjects/Packsmith-worktrees/` 下（命名如 `Packsmith-worktrees/ticket-07-build-execution`），不要放在仓库同级目录。分支随票命名 `ticket/NN-slug`，基于 `integration/packsmith-v1`。
- **探索笔记**：实现前先读 `/Users/wyy/RustRoverProjects/Packsmith-notes/` 中对应主题的笔记（仓库外共享目录）。

## Agent skills

### Issue tracker

Issues live in GitHub Issues (`Suyaj/Packsmith`), managed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default canonical triage labels (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `GLOSSARY.md` + `docs/adr/`. See `docs/agents/domain.md`.
