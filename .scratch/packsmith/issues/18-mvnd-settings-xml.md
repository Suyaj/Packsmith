# 18: mvnd 自定义 settings.xml（私服依赖）

**What to build:** 支持私服依赖：设置页新增"mvnd settings.xml"卡片——多行编辑器（粘贴/编辑 XML 内容），保存写入 `<appdata>/mvnd-settings.xml`；支持清空（恢复默认，即不传 -s）。构建执行时若该文件存在，自动在构建命令追加 `-s <appdata>/mvnd-settings.xml`（用户命令里已手写 `-s` 时不重复追加）。全局一份（项目级覆盖后续再加）。中文 i18n。

**Blocked by:** None（基于集成 tip）

**Status:** resolved

> 实现：内容存加密库 `app_meta` 键 `mvnd_settings_xml`（ADR-0002），命令层保存/清空时同步物化/移除 `<appdata>/mvnd-settings.xml`（`set_mvnd_settings_xml` / `clear_mvnd_settings_xml` / `get_mvnd_settings_xml`，内容为配置非机密、明文回传编辑回填）；`build_spawn_spec` 增加 `settings_xml: Option<&Path>` 参数，文件已物化且用户命令未手写 `-s`/`--settings`（含 `-s=`/`--settings=` 附着写法）时在依赖仓库参数之后追加 `-s <path>`，文件缺失时参数不变。设置页新增等宽多行编辑卡片（保存/清空、已设置/未设置指示，中文 i18n）。

- [x] 设置页可编辑/保存/清空 settings.xml 内容，持久化到 `<appdata>/mvnd-settings.xml`
- [x] 文件存在时构建自动追加 `-s`；用户命令已含 `-s` 不重复
- [x] 文件不存在/清空后构建不追加（行为与现在一致）
- [x] 测试：spawn 参数含/不含 `-s` 两分支、编辑器保存/清空、已有 `-s` 不覆盖
