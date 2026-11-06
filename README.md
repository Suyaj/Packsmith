<div align="center">

<img src="src-tauri/icons/icon.png" width="128" alt="Packsmith 图标"/>

# Packsmith

**桌面打包部署一体机：从 git 到服务器，一键完成**

从 git 获取项目 → 临时工作区打包 → 构件入库 → 上传服务器 → 重启服务。

[技术栈](#技术栈) · [功能总览](#功能总览) · [快速开始](#快速开始) · [项目结构](#项目结构) · [文档](#文档)

</div>

---

## 简介

Packsmith 是一个面向 Java 团队的打包部署桌面应用（当前支持 Maven 项目，架构上预留多项目类型扩展）。它把"拉代码、打包、传服务器、重启服务"这条日常链路收进一个应用里：

- **环境自持**：git / JDK / mvnd 等工具链由应用托管下载，与系统全局环境完全隔离，换机器即用
- **依赖隔离**：构建依赖走应用专属的本地仓库，不污染用户 `~/.m2`，缓存持久复用
- **数据加密**：服务器凭据、Git 账号等敏感数据存于 SQLCipher 加密库

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面框架 | Tauri 2（Rust 后端 + 系统 WebView） |
| 前端 | React 19 + TypeScript + Vite |
| UI | Tailwind CSS v4 + shadcn-ui |
| 存储 | rusqlite + SQLCipher（加密库 + 密钥文件） |
| 网络/执行 | git2（进程内 git）、russh + russh-sftp（SSH/SFTP） |
| 国际化 | i18next（框架已接入，当前仅中文资源） |

## 功能总览

### 项目管理
- 通过 **git 拉取**项目（支持账密凭据，进程内 git 不依赖系统 PATH）
- 打包时**分支模糊搜索**选择

### 打包（Java/Maven）
- **托管工具链**：git / JDK 8 / mvnd 检测、下载、版本管理（设置页可视化）
- 临时工作区：每次打包自动创建、结束即销毁
- 构建实时日志 + 取消
- 展示所选分支**最新 3 条提交**（短 SHA / 作者 / 时间 / 说明）
- **自定义 mvnd settings.xml**：支持私服；HTTP 私服仓库自动放行（绕过 Maven 3.8+ 的 HTTP 拦截器）
- 构件（jar/war）自动抢救入库；多产物时**可搜索多选**保存默认构件列表

### 部署
- **部署目标**：项目 × 服务器的绑定，携带默认上传目录、映射行预设、启动/停止命令组
- **部署操作四种**：重启（默认，= 上传 + 停止 + 启动）/ 仅上传 / 仅停止 / 仅启动
- 部署对话框**自由勾选**本次要上传的构件（模糊搜索过滤），映射行自动预设
- 上传逐文件展示进度；停止失败即中止重启，可单独补跑启动命令组
- **自动连锁**：构建成功后自动对开启开关的目标执行完整部署（按映射行预设解析）
- SSH 连接 15 秒超时，连不上明确报错不干等

### 服务器管理
- SSH 服务器增删改查、**连接测试**（连通性 / 认证）
- 密码 / 私钥凭据加密存储，测试时凭据不出后端

### 历史
- 构建历史：持久化日志查看、入库构件列表、按项目筛选、单条删除/清理
- 部署历史：每次部署的目标、操作类型、结果

### 设置
- 工具链状态检测与一键下载
- Git 账号（拉取私有仓库时提前登录）
- mvnd settings.xml 编辑器（内容存加密库，物化文件自动注入 HTTP 放行块）
- 构建历史保留策略
- 关闭窗口行为：最小化到托盘 或 退出应用
- 系统托盘

## 快速开始

环境要求：Node.js ≥ 20、Rust（含 cargo）、各平台 Tauri 依赖（见 [Tauri 文档](https://v2.tauri.app/start/prerequisites/)）。

```bash
npm install

# 开发调试（启动桌面应用）
npm run tauri dev

# 仅前端热更（浏览器预览，无 Tauri 外壳）
npm run dev

# 测试与构建
npm test          # vitest 单元测试（前端）
npm run build     # tsc + vite 前端构建
cd src-tauri
cargo check       # Rust 静态检查
cargo test        # Rust 测试（含 SSH/sshd 集成测试）
```

首次启动后，到 **设置页** 下载托管工具链（git / JDK / mvnd），然后即可添加项目开始打包。

## 项目结构

```
├── src/                    # 前端（React + TS）
│   ├── pages/              # 首页 / 项目 / 服务器 / 历史 / 设置
│   ├── components/         # 打包面板、部署对话框、目标面板等
│   ├── lib/                # Tauri invoke 封装（API 边界）
│   └── i18n/               # 中文资源
├── src-tauri/              # Rust 后端
│   └── src/
│       ├── store/          # SQLCipher 加密库 + 迁移（v1–v15）
│       ├── toolchain/      # 托管工具链检测/下载/状态
│       ├── build/          # 打包流程（克隆 → 构建 → 产物入库）
│       ├── deploy/         # 部署引擎（阶段编排 + 事件流）
│       ├── targets/        # 部署目标（映射行 CRUD）
│       ├── server/         # SSH 服务器与凭据
│       └── git_auth.rs     # git 认证回调
├── GLOSSARY.md             # 领域词汇表（13+ 词条）
└── docs/adr/               # 架构决策记录（隔离/加密/密钥文件）
```

## 文档

- [GLOSSARY.md](GLOSSARY.md) — 领域词汇表：项目、打包、构件、部署目标、映射行、命令组、部署操作……
- [docs/adr/](docs/adr/) — 关键决策：三重隔离（工具链/依赖/临时工作区）、SQLCipher 加密存储、密钥文件而非钥匙串
- `.scratch/packsmith/` — 演进规格与 26 张已交付工单（本地工单追踪）

## 测试基线

Rust 约 270 个用例（含 fake-sshd 部署集成测试、迁移回滚模拟）+ 前端 153 个用例，全部通过；`cargo check` 零警告。

## License

本项目基于 [GPL-3.0](LICENSE) 开源发布。
