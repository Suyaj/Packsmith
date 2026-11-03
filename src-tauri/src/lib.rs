mod settings_commands;
mod toolchain;
mod tray;

use tauri::Manager;

pub mod build;
pub mod commands;
pub mod deploy;
pub mod git_auth;
pub mod server;
pub mod settings;
pub mod store;
pub mod targets;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(toolchain::DownloadGuard::default())
        .manage(commands::CloneRegistry::default())
        .manage(build::commands::BuildRegistry::default())
        .manage(deploy::commands::DeployRegistry::default())
        .invoke_handler(tauri::generate_handler![
            toolchain::toolchain_status,
            toolchain::toolchain_download,
            commands::list_projects,
            commands::clone_project,
            commands::import_project,
            commands::cancel_clone,
            build::commands::list_remote_branches,
            build::commands::start_build_workspace,
            build::commands::cancel_build_workspace,
            build::commands::start_build,
            build::commands::update_build_command,
            build::commands::read_build_log,
            build::commands::list_build_history,
            build::commands::list_project_build_history,
            build::commands::list_build_artifacts,
            build::commands::delete_build_history,
            build::commands::cleanup_project_history,
            build::commands::get_build_retention,
            build::commands::set_build_retention,
            build::commands::set_default_artifact,
            server::commands::list_servers,
            server::commands::create_server,
            server::commands::update_server,
            server::commands::delete_server,
            server::commands::test_server_connection,
            server::commands::test_saved_server_connection,
            server::commands::accept_host_key,
            server::commands::check_server_reachable,
            server::commands::list_server_last_deploys,
            targets::commands::list_deployment_targets,
            targets::commands::create_deployment_target,
            targets::commands::update_deployment_target,
            targets::commands::delete_deployment_target,
            targets::commands::set_target_auto_deploy,
            targets::commands::set_target_last_artifact,
            deploy::commands::start_deploy,
            deploy::commands::cancel_deploy,
            deploy::commands::run_start_group,
            settings_commands::get_close_window_behavior,
            settings_commands::set_close_window_behavior,
            settings_commands::get_git_account,
            settings_commands::set_git_account,
            settings_commands::clear_git_account,
            settings_commands::get_mvnd_settings_xml,
            settings_commands::set_mvnd_settings_xml,
            settings_commands::clear_mvnd_settings_xml,
        ])
        .on_window_event(|window, event| {
            // 关窗行为随设置生效（ticket 11）：最小化到托盘 = 拦截关闭并隐藏；
            // 退出应用 = 放行关闭。托盘菜单的"退出应用"不经过这里，始终退出。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let behavior = app
                    .try_state::<store::Store>()
                    .and_then(|store| settings::get_close_window_behavior(&store).ok())
                    .unwrap_or_default();
                if settings::close_action(behavior) == settings::CloseAction::HideToTray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            // Auto-initialize the encrypted store on every startup: the key
            // lives in <appdata>/db.key (0600), generated on first run; the
            // keychain is only touched once to migrate pre-ticket-16 keys —
            // the user is never prompted for input.
            let db_path = app.path().app_data_dir()?.join("packsmith.db");
            let store = store::Store::open_default(&db_path)?;
            app.manage(store);

            // 系统托盘常驻（ticket 11）：打开主界面 / 退出应用。
            tray::build_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
