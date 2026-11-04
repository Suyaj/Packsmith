# 19: 部署目标上传目录表格化——文件名→目录映射

**What to build:** 部署目标编辑界面的"上传目录"改为**表格**：每行 = {构建产物文件名 → 服务器上传目录}，可增删行。部署时按行自动匹配本次构建的产物并上传到对应目录；产物缺失的行标红提示且不上传；**取代票 17 的部署时选文件**（部署对话框变为"确认解析后的上传计划"：每行复选框默认勾选，缺失行禁用+警告；自动连锁只上传可解析行）。旧数据迁移：单个上传目录 → 一行 fileName 为空的行，UI 显示「（未指定文件）」，部署时需补选（对话框内联选择器）；自动连锁跳过空名行并提示。

**Blocked by:** None（基于集成 tip）

**Status:** resolved

> 实现（续接一次中断的运行，后端/前端均为续写后重验）：
> - **存储**（迁移 v12）：新子表 `deployment_target_files(id, target_id FK 级联, file_name, upload_dir, position)`；旧目标各回填一行 `{file_name: '', upload_dir: 旧目录}`。`deployment_targets.upload_dir` 列保留但废弃（不迁移业务数据）；`last_artifact`（票 17）同样废弃保留。
> - **目标层**：`list_files` / `files_for_targets` / `replace_files`（整组原子替换；目录须单行非空、同目标内具名文件名不得重复、空名行可多个）；`create/update_deployment_target` 随表单整组携带 `files`；错误以稳定 code 暴露（`empty_upload_dir` / `upload_dir_multiline` / `duplicate_file_name`）。
> - **部署引擎**：`TargetJob.upload_dir/artifact` 改为 `artifacts: Vec<ArtifactRef>`（每行自带目录）；一行表照常停止→启动各一次；新增 `DeployEvent::Skipped` 提示事件。
> - **手动部署**（取代票 17 的部署时选文件）：对话框变为"确认解析后的上传计划"——逐目标展示行（默认勾选；缺失行禁用+标红警告；空名行「（未指定文件）」内联下拉补选，默认预选项目默认构件/首个产物），上传集合 = 勾选集合；具名行以后端库内文件名为准；提交行 id 校验属主与去重（`invalid_row`）；全无可上传行拒绝（`no_files_selected`）。
> - **自动连锁**：lenient 解析——缺失行/空名行跳过并逐条 `Skipped` 提示（空名行在无对话框处不可补选）；无任何可解析行的目标整体跳过（绝不无新产物重启）；全部目标无行则为 no-op。不再使用单产物/默认产物推断（票 10 的该逻辑随票 17 一起废止）。
> - **前端**：目标面板映射表（增删行、空名行占位提示、重复文件名校验）；部署对话框上传计划表 + skipped 提示列表；`targetsApi`/`deployApi` payload 换为 `files[]` / `files: [{rowId, fileName}]`；移除 `setTargetLastArtifact` 前端调用。中文 i18n、GLOSSARY「部署目标/映射行」词条更新。

- [x] 目标编辑界面表格：多行 {文件名→目录} 增删改，保存入加密库（迁移 v12 子表，FK 级联）
- [x] 旧单目录迁移为一行空文件名行，既有部署目标不丢
- [x] 部署对话框：逐目标展示解析后的行（可勾选/禁用+警告），空名行内联补选；上传集合=勾选集合
- [x] 产物缺失行标红不上传；自动连锁只传可解析行，空名行跳过有提示
- [x] 停止→启动编排、取消、进度、停止失败中止全部不回退（既有测试仍绿）
- [x] 测试：映射解析、缺失行跳过、迁移、对话框提交 payload、自动连锁行为
