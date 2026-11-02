use tauri::Manager;

pub mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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
