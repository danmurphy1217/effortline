use crate::diagnostics::{Diagnostics, Event};
use crate::fit_preview::{PendingPreview, PreviewState};
use crate::library_secret::KeychainSecret;
use effortline_core::library::{
    ActivityLibrary, ImportStatus, LibraryError, LibraryProgress, LibrarySecretProvider,
    LibraryStage,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct LibraryState(pub Mutex<Option<ActivityLibrary>>);
use tauri::{ipc::Channel, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SaveStage {
    KeychainAccess,
    OpenRecovery,
    FitParsing,
    DuplicateCheck,
    Encryption,
    FileWriteSync,
    SampleInserts,
    DatabaseCommit,
}

impl From<LibraryStage> for SaveStage {
    fn from(stage: LibraryStage) -> Self {
        match stage {
            LibraryStage::SecretAccess => Self::KeychainAccess,
            LibraryStage::OpenRecovery => Self::OpenRecovery,
            LibraryStage::FitParsing => Self::FitParsing,
            LibraryStage::DuplicateCheck => Self::DuplicateCheck,
            LibraryStage::Encryption => Self::Encryption,
            LibraryStage::FileWriteSync => Self::FileWriteSync,
            LibraryStage::SampleInserts => Self::SampleInserts,
            LibraryStage::DatabaseCommit => Self::DatabaseCommit,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SaveProgress {
    SamplesWritten {
        version: u8,
        completed: usize,
        total: usize,
        elapsed_ms: f64,
    },
    Started {
        version: u8,
        stage: SaveStage,
    },
    Finished {
        version: u8,
        stage: SaveStage,
        elapsed_ms: f64,
        succeeded: bool,
    },
}

impl From<LibraryProgress> for SaveProgress {
    fn from(event: LibraryProgress) -> Self {
        match event {
            LibraryProgress::SamplesWritten {
                completed,
                total,
                elapsed,
            } => Self::SamplesWritten {
                version: 1,
                completed,
                total,
                elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            },
            LibraryProgress::Started(stage) => Self::Started {
                version: 1,
                stage: stage.into(),
            },
            LibraryProgress::Finished {
                stage,
                elapsed,
                succeeded,
            } => Self::Finished {
                version: 1,
                stage: stage.into(),
                elapsed_ms: elapsed.as_secs_f64() * 1000.0,
                succeeded,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SaveRequest {
    version: u8,
    preview_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SaveResponse {
    Saved { version: u8 },
    AlreadyPresent { version: u8 },
    Error { version: u8, code: SaveError },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SaveError {
    UnsupportedRequest,
    PreviewExpired,
    InvalidPreview,
    LibraryLocationUnavailable,
    LibrarySecretUnavailable,
    LibraryBusy,
    LibraryCannotUnlock,
    LibraryEncryptionUnavailable,
    LibraryUnsupportedSchema,
    LibraryIncomplete,
    LibraryCorruptOriginal,
    LibraryRandomnessUnavailable,
    LibraryIo,
    LibraryDatabase,
    SaveFailed,
}

impl From<LibraryError> for SaveError {
    fn from(error: LibraryError) -> Self {
        match error {
            LibraryError::Import(_) => Self::InvalidPreview,
            LibraryError::SecretUnavailable => Self::LibrarySecretUnavailable,
            LibraryError::Busy => Self::LibraryBusy,
            LibraryError::CannotUnlock => Self::LibraryCannotUnlock,
            LibraryError::EncryptionUnavailable => Self::LibraryEncryptionUnavailable,
            LibraryError::UnsupportedSchema => Self::LibraryUnsupportedSchema,
            LibraryError::Incomplete => Self::LibraryIncomplete,
            LibraryError::CorruptOriginal => Self::LibraryCorruptOriginal,
            LibraryError::Randomness => Self::LibraryRandomnessUnavailable,
            LibraryError::Io(_) => Self::LibraryIo,
            LibraryError::Database(_) => Self::LibraryDatabase,
        }
    }
}

impl SaveResponse {
    fn error(code: SaveError) -> Self {
        Self::Error { version: 1, code }
    }
}

#[tauri::command]
pub(super) async fn save_preview_to_library(
    app: tauri::AppHandle,
    request: SaveRequest,
    on_progress: Channel<SaveProgress>,
) -> SaveResponse {
    let diagnostics = app.state::<Diagnostics>();
    let (operation, started) = diagnostics.begin(Event::SaveStarted);
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let diagnostics = app.state::<Diagnostics>();
        let state = app.state::<PreviewState>();
        let Ok(pending) = state.0.try_lock() else {
            return SaveResponse::error(SaveError::LibraryBusy);
        };
        let directory = match app.path().app_local_data_dir() {
            Ok(path) => path.join("library"),
            Err(_) => return SaveResponse::error(SaveError::LibraryLocationUnavailable),
        };
        let provider = KeychainSecret {
            directory: &directory,
            service: "com.danmurphy.effortline.library.v1",
            account: "primary",
        };
        let library_state = app.state::<LibraryState>();
        let Ok(mut library) = library_state.0.try_lock() else {
            return SaveResponse::error(SaveError::LibraryBusy);
        };
        if library.is_some() {
            diagnostics.record(operation, Event::LibraryReused);
        }
        save_in_session(
            &mut library,
            pending.as_ref(),
            &request,
            &directory,
            &provider,
            &mut |event| {
                // A closed UI must not abort an in-flight durable write.
                let _ = on_progress.send(SaveProgress::from(event));
                diagnostics.record(
                    operation,
                    Event::SaveProgress {
                        progress: event.into(),
                    },
                );
            },
        )
    })
    .await
    .unwrap_or_else(|_| SaveResponse::error(SaveError::SaveFailed));
    diagnostics.record(
        operation,
        Event::SaveFinished {
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            result: result.clone(),
        },
    );
    result
}

#[cfg(test)]
fn save_pending_preview(
    pending: Option<&PendingPreview>,
    request: &SaveRequest,
    directory: &Path,
    provider: &impl LibrarySecretProvider,
) -> SaveResponse {
    save_pending_preview_with_progress(pending, request, directory, provider, &mut |_| {})
}

#[cfg(test)]
fn save_pending_preview_with_progress(
    pending: Option<&PendingPreview>,
    request: &SaveRequest,
    directory: &Path,
    provider: &impl LibrarySecretProvider,
    report: &mut impl FnMut(LibraryProgress),
) -> SaveResponse {
    save_in_session(&mut None, pending, request, directory, provider, report)
}

fn save_in_session(
    session: &mut Option<ActivityLibrary>,
    pending: Option<&PendingPreview>,
    request: &SaveRequest,
    directory: &Path,
    provider: &impl LibrarySecretProvider,
    report: &mut impl FnMut(LibraryProgress),
) -> SaveResponse {
    if request.version != 1 {
        return SaveResponse::error(SaveError::UnsupportedRequest);
    }
    let Some(pending) = pending.filter(|preview| preview.id == request.preview_id) else {
        return SaveResponse::error(SaveError::PreviewExpired);
    };
    if session.is_none() {
        match ActivityLibrary::open_with_progress(directory, provider, report) {
            Ok(library) => *session = Some(library),
            Err(error) => return SaveResponse::error(error.into()),
        }
    }
    // Take ownership during the operation. Any failed write drops the connection and lock;
    // the next attempt must open and recover before writing again.
    let Some(mut library) = session.take() else {
        return SaveResponse::error(SaveError::SaveFailed);
    };
    match library.import_fit_bytes_with_progress(&pending.bytes, report) {
        Ok(result) => {
            *session = Some(library);
            match result.status {
                ImportStatus::Saved => SaveResponse::Saved { version: 1 },
                ImportStatus::AlreadyPresent => SaveResponse::AlreadyPresent { version: 1 },
            }
        }
        Err(error) => SaveResponse::error(error.into()),
    }
}

#[cfg(test)]
#[path = "../../../../crates/effortline-core/tests/support/mod.rs"]
mod synthetic_support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit_preview::{
        preview_from_reader, preview_selected_path, tests::synthetic_fit, PreviewResponse,
    };
    use effortline_core::fit_import::import_fit_activity;
    use effortline_core::library::{LibrarySecret, SecretUnavailable};
    use std::io::Cursor;

    struct TestSecret;
    impl LibrarySecretProvider for TestSecret {
        fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
            Ok(LibrarySecret::from_bytes([7; 32]))
        }
    }

    fn preview(bytes: &[u8]) -> (Option<PendingPreview>, SaveRequest) {
        let mut pending = None;
        let PreviewResponse::Ready { preview_id, .. } =
            preview_from_reader(Cursor::new(bytes), &mut pending)
        else {
            panic!("synthetic preview should be ready");
        };
        (
            pending,
            SaveRequest {
                version: 1,
                preview_id,
            },
        )
    }

    #[test]
    fn saves_exact_preview_bytes_and_retries_without_duplicates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.fit");
        let bytes = synthetic_fit(true);
        std::fs::write(&path, &bytes).unwrap();
        let mut pending = None;
        let PreviewResponse::Ready { preview_id, .. } =
            preview_selected_path(Some(path.clone()), &mut pending)
        else {
            panic!("synthetic preview should be ready");
        };
        std::fs::write(path, synthetic_fit(false)).unwrap();
        let request = SaveRequest {
            version: 1,
            preview_id,
        };
        let library_path = directory.path().join("library");
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &library_path, &TestSecret),
            SaveResponse::Saved { version: 1 }
        );
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &library_path, &TestSecret),
            SaveResponse::AlreadyPresent { version: 1 }
        );
        let library = ActivityLibrary::open(&library_path, &TestSecret).unwrap();
        let expected = import_fit_activity(&bytes).unwrap();
        assert_eq!(
            library.find_activity(&expected.source.identity).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            library
                .read_original_bytes(&expected.source.identity)
                .unwrap(),
            Some(bytes)
        );
        assert_eq!(library.list_activity_ids().unwrap().len(), 1);
    }

    #[test]
    fn cancelled_replaced_and_invalid_requests_cannot_save_or_access_secrets() {
        struct MustNotLoad;
        impl LibrarySecretProvider for MustNotLoad {
            fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
                panic!("invalid request must not access secrets");
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unused");
        let (mut pending, mut request) = preview(&synthetic_fit(true));
        request.version = 2;
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &MustNotLoad),
            SaveResponse::error(SaveError::UnsupportedRequest)
        );
        request.version = 1;
        preview_from_reader(Cursor::new(synthetic_fit(false)), &mut pending);
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &MustNotLoad),
            SaveResponse::error(SaveError::PreviewExpired)
        );
        preview_selected_path(None, &mut pending);
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &MustNotLoad),
            SaveResponse::error(SaveError::PreviewExpired)
        );
        assert!(!path.exists());
    }

    #[test]
    fn denied_secret_access_keeps_preview_for_retry_and_returns_safe_code() {
        struct Denied;
        impl LibrarySecretProvider for Denied {
            fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
                Err(SecretUnavailable)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library");
        let (pending, request) = preview(&synthetic_fit(true));
        let response = save_pending_preview(pending.as_ref(), &request, &path, &Denied);
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({"version":1,"status":"error","code":"library_secret_unavailable"})
        );
        assert!(!path.exists());
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &TestSecret),
            SaveResponse::Saved { version: 1 }
        );
        assert!(serde_json::from_value::<SaveRequest>(
            serde_json::json!({"version":1,"preview_id":request.preview_id,"path":"untrusted"})
        )
        .is_err());
    }

    #[test]
    fn keychain_stage_reaches_client_before_secret_access_returns() {
        use std::sync::mpsc;
        use std::time::Duration;

        struct WaitingSecret(mpsc::Receiver<()>);
        impl LibrarySecretProvider for WaitingSecret {
            fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
                self.0.recv_timeout(Duration::from_secs(5)).unwrap();
                Err(SecretUnavailable)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library");
        let (pending, request) = preview(&synthetic_fit(true));
        let (release, wait) = mpsc::channel();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            save_pending_preview_with_progress(
                pending.as_ref(),
                &request,
                &path,
                &WaitingSecret(wait),
                &mut |event| {
                    send.send(serde_json::to_value(SaveProgress::from(event)).unwrap())
                        .unwrap()
                },
            )
        });
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(5)).unwrap(),
            serde_json::json!({"version":1,"status":"started","stage":"keychain_access"})
        );
        assert!(!worker.is_finished());
        release.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap(),
            SaveResponse::error(SaveError::LibrarySecretUnavailable)
        );
        let finished = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(finished["stage"], "keychain_access");
        assert_eq!(finished["succeeded"], false);
        assert!(receive.try_recv().is_err());
        assert!(!directory.path().join("library").exists());
    }

    #[test]
    fn repeated_saves_reuse_prepopulated_library_and_recover_after_failure_or_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library");
        let mut initial = ActivityLibrary::open(&path, &TestSecret).unwrap();
        for count in 1..=64 {
            initial
                .import_fit_bytes(&synthetic_support::synthetic_fit(4, true, count))
                .unwrap();
        }
        drop(initial);
        let mut session = None;
        let mut events = Vec::new();
        let (pending, request) = preview(&synthetic_support::synthetic_fit(4, true, 65));
        assert_eq!(
            save_in_session(
                &mut session,
                pending.as_ref(),
                &request,
                &path,
                &TestSecret,
                &mut |event| events.push(event)
            ),
            SaveResponse::Saved { version: 1 }
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, LibraryProgress::Started(LibraryStage::OpenRecovery))));
        struct MustNotLoad;
        impl LibrarySecretProvider for MustNotLoad {
            fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
                panic!("session must reuse its open library");
            }
        }
        events.clear();
        let (pending, request) = preview(&synthetic_support::synthetic_fit(4, true, 66));
        assert_eq!(
            save_in_session(
                &mut session,
                pending.as_ref(),
                &request,
                &path,
                &MustNotLoad,
                &mut |event| events.push(event)
            ),
            SaveResponse::Saved { version: 1 }
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            LibraryProgress::Started(LibraryStage::SecretAccess | LibraryStage::OpenRecovery)
        )));
        assert_eq!(
            session.as_ref().unwrap().list_activity_ids().unwrap().len(),
            66
        );
        assert!(matches!(
            ActivityLibrary::open(&path, &TestSecret),
            Err(LibraryError::Busy)
        ));
        assert_eq!(
            save_in_session(
                &mut session,
                pending.as_ref(),
                &request,
                &path,
                &MustNotLoad,
                &mut |_| {}
            ),
            SaveResponse::AlreadyPresent { version: 1 }
        );

        // An error invalidates the session. Next open must clean interrupted writes.
        let broken = PendingPreview {
            id: request.preview_id.clone(),
            bytes: b"synthetic invalid".to_vec().into(),
        };
        assert_eq!(
            save_in_session(
                &mut session,
                Some(&broken),
                &request,
                &path,
                &MustNotLoad,
                &mut |_| {}
            ),
            SaveResponse::error(SaveError::InvalidPreview)
        );
        assert!(session.is_none());
        for restart in [false, true] {
            if restart {
                drop(session.take());
            }
            let orphan = path.join("objects/00000000000000000000000000000000.fitenc");
            std::fs::write(&orphan, b"synthetic interrupted write").unwrap();
            events.clear();
            assert_eq!(
                save_in_session(
                    &mut session,
                    pending.as_ref(),
                    &request,
                    &path,
                    &TestSecret,
                    &mut |event| events.push(event)
                ),
                SaveResponse::AlreadyPresent { version: 1 }
            );
            assert!(events.iter().any(|event| matches!(
                event,
                LibraryProgress::Started(LibraryStage::OpenRecovery)
            )));
            assert!(!orphan.exists());
            assert_eq!(
                session.as_ref().unwrap().list_activity_ids().unwrap().len(),
                66
            );
        }
    }

    #[test]
    fn large_save_reports_uncommitted_rows_while_persistence_is_running() {
        use std::sync::mpsc;
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library");
        let (pending, request) = preview(&synthetic_support::synthetic_fit(4, true, 16_705));
        let (send, receive) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut samples = Vec::new();
            let mut session = None;
            let result = save_in_session(
                &mut session,
                pending.as_ref(),
                &request,
                &path,
                &TestSecret,
                &mut |event| {
                    if let LibraryProgress::SamplesWritten {
                        completed, total, ..
                    } = event
                    {
                        samples.push(completed);
                        assert_eq!(total, 16_705);
                        if completed == 1024 {
                            send.send(serde_json::to_value(SaveProgress::from(event)).unwrap())
                                .unwrap();
                            // Handshake, not a timing assertion: hold real persistence between chunks.
                            wait.recv_timeout(Duration::from_secs(60)).unwrap();
                        }
                    }
                },
            );
            assert_eq!(result, SaveResponse::Saved { version: 1 });
            assert_eq!(samples.last(), Some(&16_705));
            assert!(samples.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(samples.len() <= 101);
        });
        let progress = receive.recv_timeout(Duration::from_secs(60)).unwrap();
        assert_eq!(progress["status"], "samples_written");
        assert_eq!(progress["completed"], 1024);
        assert_eq!(progress["total"], 16_705);
        assert!(!worker.is_finished());
        release.send(()).unwrap();
        worker.join().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "uses the macOS login Keychain; run explicitly on a development Mac"]
    fn live_keychain_preview_save_reopen_and_missing_key() {
        use security_framework::os::macos::keychain::SecKeychain;
        let directory = tempfile::tempdir().unwrap();
        let service = format!(
            "com.danmurphy.effortline.test.{}",
            directory.path().file_name().unwrap().to_str().unwrap()
        );
        let path = directory.path().join("library");
        let provider = KeychainSecret {
            directory: &path,
            service: &service,
            account: "synthetic",
        };
        // Remove only the disposable test entry, including on assertion failure.
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(keychain) = SecKeychain::default() {
                    if let Ok((_, item)) = keychain.find_generic_password(&self.0, "synthetic") {
                        item.delete();
                    }
                }
            }
        }
        let _cleanup = Cleanup(service.clone());
        let bytes = synthetic_fit(true);
        let (pending, request) = preview(&bytes);
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &provider),
            SaveResponse::Saved { version: 1 }
        );
        let library = ActivityLibrary::open(&path, &provider).unwrap();
        let identity = import_fit_activity(&bytes).unwrap().source.identity;
        assert_eq!(library.read_original_bytes(&identity).unwrap(), Some(bytes));
        drop(library);
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &provider),
            SaveResponse::AlreadyPresent { version: 1 }
        );
        let keychain = SecKeychain::default().unwrap();
        let (password, item) = keychain
            .find_generic_password(&service, "synthetic")
            .unwrap();
        assert_eq!(password.len(), 32);
        drop(password);
        item.delete();
        assert_eq!(
            save_pending_preview(pending.as_ref(), &request, &path, &provider),
            SaveResponse::error(SaveError::LibrarySecretUnavailable)
        );
        assert!(keychain
            .find_generic_password(&service, "synthetic")
            .is_err());
    }
}
