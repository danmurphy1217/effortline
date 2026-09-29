mod fit_preview;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![fit_preview::preview_fit_activity])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
