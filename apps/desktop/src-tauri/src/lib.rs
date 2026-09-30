mod fit_preview;
mod library_save;
mod library_secret;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(fit_preview::PreviewState::default())
        .invoke_handler(tauri::generate_handler![
            fit_preview::preview_fit_activity,
            library_save::save_preview_to_library
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
