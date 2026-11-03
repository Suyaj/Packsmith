mod toolchain;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(toolchain::DownloadGuard::default())
        .invoke_handler(tauri::generate_handler![
            toolchain::toolchain_status,
            toolchain::toolchain_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
