# 15: 修复托管工具链解压丢失执行位（构建 spawn Permission denied）

**What to build:** 用户实测：zip 安装的托管 mvnd/JDK 所有二进制权限为 0644，构建 spawn 报 `Permission denied (os error 13)`。根因（诊断已证实）：`toolchain/archive.rs` 的 `extract_zip` 不应用 zip 条目的 `unix_mode`（tar.gz 路径不受影响）；且安装校验 `find_executable_in` 只查 `is_file()` 不查执行位，坏安装未被拦截。

修复三层：
1. **回归测试**（先红后绿）：zip 夹具带 `unix_mode: 0o755` 的可执行条目 → `extract_archive` → 断言落盘文件保留执行位
2. **extract_zip 修复**：解压时应用条目 `unix_mode()`（存在才应用，Windows 打包的 zip 可能没有；维持既有 zip-slip 防御不变）
3. **安装校验加固**：注册可执行文件后校验其有执行位；无则 `chmod 0o755` 自愈（覆盖无模式信息的包），自愈失败则安装报错——不再让坏安装流到 spawn 阶段

**Blocked by:** None（基于集成 tip）

**Status:** resolved
<!-- 诊断：extract_zip 丢弃 zip 条目 unix_mode（一律 0644）；find_executable_in 只查 is_file() 不查执行位，坏安装未被拦截。 -->

- [x] 回归测试：zip（带 unix_mode 0755）解压后执行位保留（修复前红、修复后绿）
- [x] extract_zip 应用 unix_mode；zip-slip 防御不回退（既有测试仍绿）
- [x] 安装校验：注册的可执行文件无执行位时 chmod 自愈/报错，坏安装无法产生
- [x] 全套测试绿：cargo（0 警告）/ npm / build
