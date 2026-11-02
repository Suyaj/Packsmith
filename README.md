# Packsmith

桌面打包应用：从 git 获取项目 → 临时工作区 mvnd 打包 → 构件入库 → 上传服务器 → 命令组重启服务。v1 仅 Java/Maven。

技术栈：Tauri（Rust 后端 + WebView 前端）+ React + TypeScript + Vite + Tailwind + shadcn-ui；i18n 框架已接入，v1 仅中文资源。

## 开发

```bash
npm install
npm run tauri dev     # 启动桌面应用
npm run dev           # 仅前端（浏览器）
npm test              # vitest 单元测试
npm run build         # tsc + vite 前端构建
```

Rust 侧位于 `src-tauri/`，用 `cargo check` / `cargo build` 校验。
