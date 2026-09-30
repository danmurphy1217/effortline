use crate::fit_preview::{PendingPreview, PreviewState};
use crate::library_secret::KeychainSecret;
use effortline_core::library::{
    ActivityLibrary, ImportStatus, LibraryError, LibrarySecretProvider,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Manager;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SaveRequest {
    version: u8,
    preview_id: String,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SaveResponse {
    Saved { version: u8 },
    AlreadyPresent { version: u8 },
    Error { version: u8, code: SaveError },
}

#[derive(Debug, PartialEq, Serialize)]
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
) -> SaveResponse {
    tauri::async_runtime::spawn_blocking(move || {
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
        save_pending_preview(pending.as_ref(), &request, &directory, &provider)
    })
    .await
    .unwrap_or_else(|_| SaveResponse::error(SaveError::SaveFailed))
}

fn save_pending_preview(
    pending: Option<&PendingPreview>,
    request: &SaveRequest,
    directory: &Path,
    provider: &impl LibrarySecretProvider,
) -> SaveResponse {
    if request.version != 1 {
        return SaveResponse::error(SaveError::UnsupportedRequest);
    }
    let Some(pending) = pending.filter(|preview| preview.id == request.preview_id) else {
        return SaveResponse::error(SaveError::PreviewExpired);
    };
    match ActivityLibrary::open(directory, provider)
        .and_then(|mut library| library.import_fit_bytes(&pending.bytes))
    {
        Ok(result) => match result.status {
            ImportStatus::Saved => SaveResponse::Saved { version: 1 },
            ImportStatus::AlreadyPresent => SaveResponse::AlreadyPresent { version: 1 },
        },
        Err(error) => SaveResponse::error(error.into()),
    }
}

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
