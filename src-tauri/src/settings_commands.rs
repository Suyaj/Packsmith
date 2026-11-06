//! 设置相关 Tauri commands（ticket 11：关闭窗口行为；ticket 13：Git 账号；
//! ticket 18：mvnd settings.xml；ticket 20：物化时自动注入 HTTP blocker 覆盖）。
//!
//! 跨 IPC 的值是 `'minimize' | 'quit'` 字符串（[`crate::settings`] 的持久化
//! 契约）；非法值在写入侧即被拒绝，读取侧永远返回合法值。

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::ApiError;
use crate::settings::{self, CloseWindowBehavior, CloseWindowBehaviorParseError};
use crate::store::Store;
use crate::store::settings as store_settings;

impl From<CloseWindowBehaviorParseError> for ApiError {
    fn from(e: CloseWindowBehaviorParseError) -> Self {
        ApiError::raw("invalid_close_window_behavior", e.to_string())
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(e: crate::store::StoreError) -> Self {
        ApiError::raw("store", e.to_string())
    }
}

/// 读取关闭窗口行为设置（`'minimize'`（默认）或 `'quit'`）。
#[tauri::command]
pub fn get_close_window_behavior(app: AppHandle) -> Result<String, ApiError> {
    let store = app.state::<Store>();
    Ok(settings::get_close_window_behavior(&store)?
        .as_str()
        .to_string())
}

/// 保存关闭窗口行为设置（设置页切换后立即生效——下一次关窗即按新值处置）。
#[tauri::command]
pub fn set_close_window_behavior(app: AppHandle, behavior: String) -> Result<(), ApiError> {
    let behavior: CloseWindowBehavior = behavior.parse()?;
    let store = app.state::<Store>();
    settings::set_close_window_behavior(&store, behavior)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Git 账号 (ticket 13)：clone / 分支拉取的 HTTPS 认证凭据，入加密库
// ---------------------------------------------------------------------------

/// Git 账号出参（掩码模式，沿用服务器凭据模式）：用户名非机密可直接展示；
/// 令牌只给"是否已设置"指示，**永不明文回传前端**。编辑时令牌留空 =
/// 保持不变（前端不回传），显式"清除账号"走 [`clear_git_account`]。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitAccountDto {
    pub username: Option<String>,
    pub has_token: bool,
}

/// 读取 Git 账号状态（设置页卡片展示）。
#[tauri::command]
pub fn get_git_account(app: AppHandle) -> Result<GitAccountDto, ApiError> {
    let store = app.state::<Store>();
    git_account_dto(&store)
}

/// 保存 Git 账号。
///
/// `token` 两态：字段缺席或 `null` → **保持现有令牌不变**（编辑表单留空即
/// 保持）；有值 → 更新。单独清令牌无 UI 入口——显式"清除账号"用
/// [`clear_git_account`]（用户名与令牌一起删除）。用户名必填（非空白），
/// 否则返回 `git_username_required`——不落库，现有账号原样保留。
#[tauri::command]
pub fn set_git_account(
    app: AppHandle,
    username: String,
    token: Option<String>,
) -> Result<GitAccountDto, ApiError> {
    let store = app.state::<Store>();
    apply_git_account(&store, &username, token)
}

/// 显式清除 Git 账号（用户名与令牌一起删除）。
#[tauri::command]
pub fn clear_git_account(app: AppHandle) -> Result<GitAccountDto, ApiError> {
    let store = app.state::<Store>();
    clear_git_account_in(&store)
}

fn git_account_dto(store: &Store) -> Result<GitAccountDto, ApiError> {
    let username = store_settings::get_git_username(store).map_err(ApiError::from)?;
    let has_token = store_settings::get_git_token(store)
        .map_err(ApiError::from)?
        .is_some();
    Ok(GitAccountDto { username, has_token })
}

fn apply_git_account(
    store: &Store,
    username: &str,
    token: Option<String>,
) -> Result<GitAccountDto, ApiError> {
    if username.trim().is_empty() {
        return Err(ApiError::raw(
            "git_username_required",
            "Git 用户名不能为空",
        ));
    }
    store_settings::set_git_username(store, username).map_err(ApiError::from)?;
    // token 为 None（字段缺席）：保持现有令牌不变（编辑表单令牌留空）。
    if let Some(token) = token {
        store_settings::set_git_token(store, &token).map_err(ApiError::from)?;
    }
    git_account_dto(store)
}

fn clear_git_account_in(store: &Store) -> Result<GitAccountDto, ApiError> {
    store_settings::clear_git_account(store).map_err(ApiError::from)?;
    git_account_dto(store)
}

// ---------------------------------------------------------------------------
// mvnd settings.xml (ticket 18)：私服依赖的全局 settings.xml
// ---------------------------------------------------------------------------

/// mvnd settings.xml 出参：内容是配置而非机密，可以明文回传前端做编辑回填；
/// `file_exists` 指示 `<appdata>/mvnd-settings.xml` 是否已物化（构建据此追
/// 加 `-s`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MvndSettingsXmlDto {
    pub content: Option<String>,
    pub file_exists: bool,
}

/// mvnd settings.xml 的磁盘物化路径（构建 spawn 侧按同名约定拼接）。
pub fn settings_xml_path(appdata: &Path) -> PathBuf {
    appdata.join("mvnd-settings.xml")
}

/// ticket 20：HTTP blocker 覆盖注入块——同名 id `maven-default-http-blocker`
/// 重定义 mvnd/Maven 3.8+ 全局自带的拦截镜像，`mirrorOf` 指向哑仓库且
/// `blocked=false`，使全局 HTTP 仓库拦截失效（实机已验证：注入后可从公司
/// HTTP 私服正常拉取依赖）。优先级高于用户 settings 里普通 mirror 定义。
const HTTP_BLOCKER_OVERRIDE: &str = "<mirror><id>maven-default-http-blocker</id>\
<mirrorOf>dummy-disabled-repo</mirrorOf><url>http://0.0.0.0/</url>\
<blocked>false</blocked></mirror>";

/// ASCII 大小写不敏感地在 `haystack` 中查找 `needle`（标签均为 ASCII，字节级
/// 比较即可；返回原始字符串中的字节偏移）。
fn find_ci_ascii(haystack: &str, needle: &str) -> Option<usize> {
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() {
        return Some(0);
    }
    if nb.len() > hb.len() {
        return None;
    }
    (0..=hb.len() - nb.len()).find(|&i| {
        hb[i..i + nb.len()]
            .iter()
            .zip(nb)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// ticket 20：物化前对用户 settings.xml 内容做 HTTP blocker 覆盖注入（字符串
/// 级，与现有实现一致，不引入 XML 解析器）。
///
/// - 内容已含 id 为 `maven-default-http-blocker` 的 mirror（大小写/空白不敏
///   感匹配）→ 用户已自行处理，原样返回；
/// - 含 `<mirrors>` → 在第一个开标签的 `>` 之后插入注入块；
/// - 无 `<mirrors>` → 在 `</settings>` 前插入 `<mirrors>注入块</mirrors>`；
/// - 兜底（文档化）：内容连 `</settings>` 都没有（非标准片段，如自闭合
///   `<settings/>`）→ 追加在文末。
///
/// 注入只发生在物化写入时：数据库仍存用户原文，编辑回显所见即所输。
/// 幂等——对注入结果再注入结果不变（已有同名 id 即命中第一条规则）。
fn inject_http_blocker_override(content: &str) -> String {
    // 去空白 + 小写后匹配 id 行：容忍 `<id> maven-default-http-blocker </id>` 等。
    let normalized: String = content
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if normalized.contains("id>maven-default-http-blocker") {
        return content.to_string();
    }

    if let Some(tag) = find_ci_ascii(content, "<mirrors") {
        // 开标签内部不允许出现 '>'，取其后的第一个 '>' 作为开标签结束。
        if let Some(gt) = content[tag..].find('>') {
            let at = tag + gt + 1;
            return format!("{}{}{}", &content[..at], HTTP_BLOCKER_OVERRIDE, &content[at..]);
        }
    }

    if let Some(at) = find_ci_ascii(content, "</settings") {
        return format!(
            "{}<mirrors>{}</mirrors>{}",
            &content[..at],
            HTTP_BLOCKER_OVERRIDE,
            &content[at..]
        );
    }

    format!("{content}<mirrors>{HTTP_BLOCKER_OVERRIDE}</mirrors>")
}

/// 读取 mvnd settings.xml 状态（设置页卡片展示与编辑回填）。
#[tauri::command]
pub fn get_mvnd_settings_xml(app: AppHandle) -> Result<MvndSettingsXmlDto, ApiError> {
    let store = app.state::<Store>();
    mvnd_settings_dto(&store, &app_data_dir(&app)?)
}

/// 保存 mvnd settings.xml：内容入加密库并物化到 `<appdata>/mvnd-settings.xml`。
/// 空白内容视为清除（与"清空"按钮同效）。
#[tauri::command]
pub fn set_mvnd_settings_xml(
    app: AppHandle,
    content: String,
) -> Result<MvndSettingsXmlDto, ApiError> {
    let appdata = app_data_dir(&app)?;
    let store = app.state::<Store>();
    apply_mvnd_settings(&store, &appdata, Some(&content))
}

/// 清空 mvnd settings.xml：删加密库记录并移除物化文件（恢复默认，即构建不
/// 追加 `-s`）。
#[tauri::command]
pub fn clear_mvnd_settings_xml(app: AppHandle) -> Result<MvndSettingsXmlDto, ApiError> {
    let appdata = app_data_dir(&app)?;
    let store = app.state::<Store>();
    apply_mvnd_settings(&store, &appdata, None)
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    app.path()
        .app_data_dir()
        .map_err(|e| ApiError::raw("io", e.to_string()))
}

fn mvnd_settings_dto(store: &Store, appdata: &Path) -> Result<MvndSettingsXmlDto, ApiError> {
    let content = store_settings::get_mvnd_settings_xml(store).map_err(ApiError::from)?;
    Ok(MvndSettingsXmlDto {
        content,
        file_exists: settings_xml_path(appdata).exists(),
    })
}

/// 保存/清空的统一入口（`content: None` 或空白 = 清除），保证加密库与磁盘
/// 文件两处状态同步。
fn apply_mvnd_settings(
    store: &Store,
    appdata: &Path,
    content: Option<&str>,
) -> Result<MvndSettingsXmlDto, ApiError> {
    match content {
        Some(c) if !c.trim().is_empty() => {
            store_settings::set_mvnd_settings_xml(store, c).map_err(ApiError::from)?;
            materialize_settings_xml(appdata, c)?;
        }
        _ => {
            store_settings::clear_mvnd_settings_xml(store).map_err(ApiError::from)?;
            dematerialize_settings_xml(appdata)?;
        }
    }
    mvnd_settings_dto(store, appdata)
}

/// 物化：把 settings.xml 内容写到 `<appdata>/mvnd-settings.xml`（构建执行从
/// 磁盘读取，mvnd 不认加密库）。写入的是**注入后**的内容（ticket 20：自动
/// 放行 HTTP 私服仓库），数据库仍保留用户原文。
fn materialize_settings_xml(appdata: &Path, content: &str) -> Result<(), ApiError> {
    let effective = inject_http_blocker_override(content);
    std::fs::create_dir_all(appdata)
        .and_then(|_| std::fs::write(settings_xml_path(appdata), effective))
        .map_err(|e| ApiError::raw("io", e.to_string()))
}

/// 去物化：删除物化文件；文件本来就不存在不算错误（幂等清空）。
fn dematerialize_settings_xml(appdata: &Path) -> Result<(), ApiError> {
    match std::fs::remove_file(settings_xml_path(appdata)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ApiError::raw("io", e.to_string())),
    }
}

#[cfg(test)]
mod git_account_tests {
    use super::*;
    use crate::store::settings as store_settings;

    const TEST_KEY_HEX: &str =
        "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn open(dir: &std::path::Path) -> crate::store::Store {
        crate::store::Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
    }

    #[test]
    fn apply_saves_username_and_token_and_dto_never_contains_token_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());

        let dto = apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        assert_eq!(dto.username.as_deref(), Some("octocat"));
        assert!(dto.has_token);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.contains("ghp_secret"), "令牌明文绝不回传前端: {json}");
        assert_eq!(
            store_settings::get_git_token(&store).unwrap().as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn token_field_absent_keeps_existing_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let dto = apply_git_account(&store, "renamed", None).unwrap();

        assert_eq!(dto.username.as_deref(), Some("renamed"));
        assert!(dto.has_token, "编辑留空令牌 = 保持不变");
        assert_eq!(
            store_settings::get_git_token(&store).unwrap().as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn token_null_also_keeps_existing_token() {
        // 两态契约：None（字段缺席或 null）= 保持不变；单独清令牌无 UI 入口，
        // 清除账号走 clear_git_account。
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let dto = apply_git_account(&store, "octocat", None).unwrap();

        assert!(dto.has_token);
        assert_eq!(
            store_settings::get_git_token(&store).unwrap().as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn blank_username_is_rejected_without_touching_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let err = apply_git_account(&store, "   ", None).unwrap_err();

        assert_eq!(err.code(), "git_username_required");
        // 校验失败不落库：现有账号原样保留。
        assert_eq!(store_settings::get_git_username(&store).unwrap().as_deref(), Some("octocat"));
        assert!(store_settings::get_git_token(&store).unwrap().is_some());
    }

    #[test]
    fn clear_git_account_removes_username_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let dto = clear_git_account_in(&store).unwrap();

        assert_eq!(dto.username, None);
        assert!(!dto.has_token);
        assert_eq!(store_settings::get_git_username(&store).unwrap(), None);
        assert_eq!(store_settings::get_git_token(&store).unwrap(), None);
    }
}

#[cfg(test)]
mod mvnd_settings_tests {
    use super::*;

    const TEST_KEY_HEX: &str =
        "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn open(dir: &std::path::Path) -> crate::store::Store {
        crate::store::Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
    }

    fn xml(appdata: &std::path::Path) -> String {
        std::fs::read_to_string(settings_xml_path(appdata)).unwrap()
    }

    #[test]
    fn 保存写入加密库并物化到磁盘文件() {
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let store = open(dir.path());
        let original = "<settings></settings>";

        let dto = apply_mvnd_settings(&store, &appdata, Some(original)).unwrap();

        assert_eq!(dto.content.as_deref(), Some(original));
        assert!(dto.file_exists);
        // 物化文件含注入块（票 20），DB 仍存用户原文。
        assert_eq!(
            xml(&appdata),
            format!("<settings><mirrors>{HTTP_BLOCKER_OVERRIDE}</mirrors></settings>")
        );
        assert_eq!(
            store_settings::get_mvnd_settings_xml(&store).unwrap().as_deref(),
            Some(original)
        );
    }

    #[test]
    fn 清空同时删除加密库记录与磁盘文件() {
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let store = open(dir.path());
        apply_mvnd_settings(&store, &appdata, Some("<settings/>")).unwrap();

        let dto = apply_mvnd_settings(&store, &appdata, None).unwrap();

        assert_eq!(dto.content, None);
        assert!(!dto.file_exists);
        assert!(!settings_xml_path(&appdata).exists());
        assert_eq!(store_settings::get_mvnd_settings_xml(&store).unwrap(), None);
    }

    #[test]
    fn 空白内容保存等同于清空() {
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let store = open(dir.path());
        apply_mvnd_settings(&store, &appdata, Some("<settings/>")).unwrap();

        let dto = apply_mvnd_settings(&store, &appdata, Some("   \n  ")).unwrap();

        assert_eq!(dto.content, None);
        assert!(!dto.file_exists);
    }

    #[test]
    fn 未设置过时清空幂等不报错() {
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let store = open(dir.path());

        let dto = apply_mvnd_settings(&store, &appdata, None).unwrap();

        assert_eq!(dto.content, None);
        assert!(!dto.file_exists);
    }

    #[test]
    fn 覆盖保存更新磁盘文件内容() {
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let store = open(dir.path());
        apply_mvnd_settings(&store, &appdata, Some("<settings>v1</settings>")).unwrap();

        apply_mvnd_settings(&store, &appdata, Some("<settings>v2</settings>")).unwrap();

        assert_eq!(
            xml(&appdata),
            format!("<settings>v2<mirrors>{HTTP_BLOCKER_OVERRIDE}</mirrors></settings>")
        );
    }
}

#[cfg(test)]
mod http_blocker_inject_tests {
    use super::*;

    /// 无 `<mirrors>`：注入块以 `<mirrors>…</mirrors>` 包裹后插到 `</settings>` 前。
    #[test]
    fn 无mirrors时在settings闭标签前插入注入块() {
        let content = "<?xml version=\"1.0\"?>\n<settings>\n</settings>";

        let out = inject_http_blocker_override(content);

        assert_eq!(
            out,
            "<?xml version=\"1.0\"?>\n<settings>\n<mirrors>\
             <mirror><id>maven-default-http-blocker</id>\
             <mirrorOf>dummy-disabled-repo</mirrorOf><url>http://0.0.0.0/</url>\
             <blocked>false</blocked></mirror></mirrors></settings>"
        );
    }

    /// 含 `<mirrors>`：注入块插在第一个开标签之后，不额外包裹。
    #[test]
    fn 含mirrors时注入块插在开标签后() {
        let content = "<settings><mirrors><mirror><id>internal</id></mirror></mirrors></settings>";

        let out = inject_http_blocker_override(content);

        assert_eq!(
            out,
            "<settings><mirrors><mirror><id>maven-default-http-blocker</id>\
             <mirrorOf>dummy-disabled-repo</mirrorOf><url>http://0.0.0.0/</url>\
             <blocked>false</blocked></mirror><mirror><id>internal</id></mirror>\
             </mirrors></settings>"
        );
    }

    /// 用户已自定义同名 id（大小写/空白不敏感）→ 原样物化，不重复注入。
    #[test]
    fn 已自定义blocker_id时原样返回() {
        let content =
            "<settings><mirrors><mirror><id> MAVEN-DEFAULT-HTTP-BLOCKER </id>\
             <mirrorOf>*</mirrorOf><blocked>false</blocked></mirror></mirrors></settings>";

        assert_eq!(inject_http_blocker_override(content), content);
    }

    /// 幂等：对注入结果再注入结果不变（同名 id 命中"用户已自行处理"分支）。
    #[test]
    fn 注入是幂等的() {
        let content = "<settings></settings>";

        let once = inject_http_blocker_override(content);

        assert_eq!(inject_http_blocker_override(&once), once);
    }

    /// DB 存用户原文 / 物化文件含注入块的端到端契约（含 mirrors 的形态）。
    #[test]
    fn 数据库存原文而物化文件含注入块() {
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let store = crate::store::Store::open(
            &dir.path().join("test.db"),
            "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
        )
        .unwrap();
        let original = "<settings><mirrors></mirrors></settings>";

        apply_mvnd_settings(&store, &appdata, Some(original)).unwrap();

        assert_eq!(
            crate::store::settings::get_mvnd_settings_xml(&store).unwrap().as_deref(),
            Some(original)
        );
        let on_disk = std::fs::read_to_string(settings_xml_path(&appdata)).unwrap();
        assert!(on_disk.contains(HTTP_BLOCKER_OVERRIDE));
        assert_eq!(
            on_disk,
            "<settings><mirrors><mirror><id>maven-default-http-blocker</id>\
             <mirrorOf>dummy-disabled-repo</mirrorOf><url>http://0.0.0.0/</url>\
             <blocked>false</blocked></mirror></mirrors></settings>"
        );
    }

    /// 兜底（文档化）：非标准片段无 `</settings>` 时追加在文末。
    #[test]
    fn 无settings闭标签时兜底追加文末() {
        let out = inject_http_blocker_override("<settings/>");

        assert_eq!(
            out,
            "<settings/><mirrors><mirror><id>maven-default-http-blocker</id>\
             <mirrorOf>dummy-disabled-repo</mirrorOf><url>http://0.0.0.0/</url>\
             <blocked>false</blocked></mirror></mirrors>"
        );
    }
}
