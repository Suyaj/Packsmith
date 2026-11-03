mod toolchain;

use tauri::Manager;

pub mod build;
pub mod commands;
pub mod server;
pub mod store;
pub mod targets;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(toolchain::DownloadGuard::default())
        .manage(commands::CloneRegistry::default())
        .manage(build::commands::BuildRegistry::default())
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
            server::commands::accept_host_key,
            targets::commands::list_deployment_targets,
            targets::commands::create_deployment_target,
            targets::commands::update_deployment_target,
            targets::commands::delete_deployment_target,
        ])
        .setup(|app| {
            // Auto-initialize the encrypted store on every startup: the key
            // is generated on first run and kept in the OS keychain, so the
            // user is never prompted for input.
            let db_path = app.path().app_data_dir()?.join("packsmith.db");
            let store = store::Store::open_default(&db_path)?;
            app.manage(store);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
