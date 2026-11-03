//! 系统托盘（ticket 11）：常驻图标 + 打开主界面 / 退出应用菜单。
//!
//! 托盘本身（图标、菜单事件）依赖运行中的 Tauri 应用，无法在纯 `cargo test`
//! 里驱动；可测的契约是纯函数：菜单项（id + 中文文案）与构建进行中的提示
//! 文案。托盘菜单的"退出应用"始终退出（`app.exit(0)`），不经过关窗行为设置。

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::build::commands::BuildRegistry;

/// 托盘实例 id（`tray_by_id` 更新状态时用同一 id 定位）。
pub const TRAY_ID: &str = "packsmith-main";

/// 菜单项 id：打开主界面。
pub const MENU_OPEN: &str = "open";
/// 菜单项 id：退出应用。
pub const MENU_QUIT: &str = "quit";

/// 托盘菜单契约：[(id, 文案)]。顺序即展示顺序。
pub fn menu_items() -> [(&'static str, &'static str); 2] {
    [(MENU_OPEN, "打开主界面"), (MENU_QUIT, "退出应用")]
}

/// 托盘提示文案：空闲 vs 打包进行中（构建状态的可见提示）。
pub fn tooltip_text(building: bool) -> &'static str {
    if building {
        "Packsmith — 打包进行中…"
    } else {
        "Packsmith"
    }
}

/// 显示主窗口（托盘左键点击与"打开主界面"共用）：先取消最小化，再显示、
/// 聚焦（research notes：Windows 上顺序不可换）。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 依据打包注册表刷新托盘的构建状态提示；注册表非空 = 有打包进行中。
/// 托盘缺失（如 Linux 无托盘环境）时静默跳过。
pub fn refresh_build_state(app: &AppHandle) {
    let running = app
        .try_state::<BuildRegistry>()
        .map(|registry| registry.0.lock().expect("build registry poisoned").len())
        .unwrap_or(0);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip_text(running > 0).to_string()));
    }
}

/// 构建（setup 阶段）托盘：常驻图标、双项菜单。左键点击图标 = 打开主界面
/// （Linux 不触发图标点击事件，菜单可用——可接受的降级）。
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, menu_items()[0].1, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, menu_items()[1].1, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(
            app.default_window_icon()
                .expect("app icon is bundled")
                .clone(),
        )
        .tooltip(tooltip_text(false))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            // 退出应用始终真退出，与关窗行为设置无关。
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_contract_is_open_and_quit_with_chinese_labels() {
        let items = menu_items();
        assert_eq!(items[0], (MENU_OPEN, "打开主界面"));
        assert_eq!(items[1], (MENU_QUIT, "退出应用"));
        assert_ne!(MENU_OPEN, MENU_QUIT, "menu ids must be distinct");
    }

    #[test]
    fn tooltip_shows_building_state_while_builds_run() {
        assert_eq!(tooltip_text(false), "Packsmith");
        let building = tooltip_text(true);
        assert_ne!(building, tooltip_text(false));
        assert!(building.contains("打包进行中"), "可见状态提示: {building}");
    }
}
