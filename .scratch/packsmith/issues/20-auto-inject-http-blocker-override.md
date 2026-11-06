# 20: settings.xml 自动注入 HTTP blocker 覆盖（实机踩坑自动化）

**What to build:** 用户实测踩坑：mvnd 全局自带的 `maven-default-http-blocker`（Maven 3.8+ 默认禁 HTTP 仓库）优先级高于用户 settings 里的 mirrors，私服 HTTP 仓库被拦，且报错极具迷惑性。自动化解法：`set_mvnd_settings_xml` 物化 `<appdata>/mvnd-settings.xml` 时，若用户内容中**没有** id 为 `maven-default-http-blocker` 的 mirror，则自动注入覆盖块（同名 id、`mirrorOf` 指向哑仓库、`blocked=false`），使全局拦截器失效。数据库仍存用户原始输入；注入只发生在物化写入时（用户编辑回显所见即所输）。

**注入规则（字符串级，与现有实现一致，不引入 XML 解析器）**：
- 内容含 `id>maven-default-http-blocker`（用户已自行处理）→ 原样物化
- 含 `<mirrors>` → 在其开标签后插入注入块
- 无 `<mirrors>` → 在 `</settings>` 前插入 `<mirrors>注入块</mirrors>`
- 注入块：`<mirror><id>maven-default-http-blocker</id><mirrorOf>dummy-disabled-repo</mirrorOf><url>http://0.0.0.0/</url><blocked>false</blocked></mirror>`

**实机验证已做过**（2026-11-04，同机托管 mvnd + 公司 HTTP 私服）：注入前 `dependency:get` 被 Blocked mirror 拦截；注入后同一命令成功从私服下载 jar。本票把该手工步骤自动化。

**Blocked by:** None（基于集成 tip）

**Status:** resolved

> 实现：`set_mvnd_settings_xml` 物化前经 `inject_http_blocker_override`（字符串级三分支：已含同名 id 原样 / 有 `<mirrors>` 开标签后插入 / 无则 `</settings>` 前包裹插入，兜底追加文末；幂等）；DB 存用户原文，物化文件含注入块；前端保存成功提示注明「已自动放行 HTTP 私服仓库」（zh i18n）。

- [x] 三种内容形态（有 mirrors / 无 mirrors / 用户已自定义 blocker id）物化结果正确，有单测覆盖
- [x] DB 存用户原文；物化文件含注入块；清空后文件移除（既有行为不回退）
- [x] 前端：保存成功提示可注明"已自动放行 HTTP 私服仓库"（i18n），不做更多 UI
- [x] 全套测试绿（cargo 0 警告 / npm / build）
