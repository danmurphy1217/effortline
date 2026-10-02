mod diagnostics;
use tauri::Manager;
mod batch_import;
mod fit_preview;
mod investigation;
mod library_save;
mod library_secret;
mod local_model;

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
        .manage(batch_import::BatchCancellation::default())
        .manage(local_model::LocalModelState::default())
        .manage(investigation::TrainingChatState::default())
        .invoke_handler(tauri::generate_handler![
            fit_preview::preview_fit_activity,
            library_save::save_preview_to_library,
            library_save::check_preview_in_library,
            investigation::training_chat,
            investigation::reset_training_chat,
            investigation::investigate_recent_running,
            local_model::local_model_status,
            local_model::install_local_model,
            local_model::cancel_local_model_install,
            local_model::remove_local_model,
            batch_import::import_fit_files,
            batch_import::cancel_fit_import
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
