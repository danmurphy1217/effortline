mod diagnostics;
use tauri::Manager;
mod fit_preview;
mod library_save;
mod library_secret;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let diagnostics = diagnostics::Diagnostics::new(app.path().app_log_dir().ok());
            diagnostics.record(
                0,
                diagnostics::Event::AppStarted {
                    app_version: env!("CARGO_PKG_VERSION"),
                    os: std::env::consts::OS,
                    arch: std::env::consts::ARCH,
                },
            );
            app.manage(diagnostics);
            Ok(())
        })
        .manage(fit_preview::PreviewState::default())
        .manage(library_save::LibraryState::default())
        .invoke_handler(tauri::generate_handler![
            fit_preview::preview_fit_activity,
            library_save::save_preview_to_library
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
