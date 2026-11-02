# 05: 项目管理：clone 与导入

**What to build:** 项目管理页可创建项目两种方式：填 git URL clone 到 workspace 根目录（默认 `~/Packsmith/workspace`），或导入已有本地路径。项目列表展示（名称、来源、类型），项目类型仅 Maven(Java) 可选，Gradle 等以 disabled 占位。项目记录入加密库。v1 不做 pull/分支管理（界面预留刷新按钮但不实现逻辑）。

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] 填 URL clone 成功，项目出现在列表，磁盘上有对应目录
- [ ] 导入本地路径成功，项目出现在列表
- [ ] 无效 URL / 已存在同名项目有明确错误
- [ ] 类型下拉框仅 Maven 可选，其余 disabled
