use crate::diagnostics::{Diagnostics, Event};
use crate::library_save::{
    save_bytes_in_session, BatchActivitySummary, LibraryState, SaveError, SaveProgress,
};
use crate::library_secret::KeychainSecret;
use effortline_core::fit_import::MAX_FIT_BYTES;
use effortline_core::library::ImportStatus;
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{ipc::Channel, Manager};
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

const MAX_BATCH_FILES: usize = 32;
const BATCH_VERSION: u8 = 1;

#[derive(Default)]
pub(super) struct BatchCancellation(Mutex<Option<ActiveBatch>>);

struct ActiveBatch {
    id: String,
    cancelled: Arc<AtomicBool>,
}

struct ActiveBatchGuard<'a> {
    state: &'a BatchCancellation,
    id: String,
}

impl Drop for ActiveBatchGuard<'_> {
    fn drop(&mut self) {
        clear_active_batch(self.state, &self.id);
    }
}

#[derive(Debug, Clone)]
struct SelectedFit {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BatchFileStatus {
    Saved,
    AlreadyPresent,
    Failed,
    NotImported,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BatchCommandError {
    PickerFailed,
    TooManyFiles,
    LibraryBusy,
    LibraryLocationUnavailable,
    RandomnessUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct BatchFileOutcome {
    index: usize,
    name: String,
    status: BatchFileStatus,
    code: Option<SaveError>,
    activity: Option<BatchActivitySummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum BatchResponse {
    PickerCancelled {
        version: u8,
    },
    Error {
        version: u8,
        code: BatchCommandError,
    },
    Completed {
        version: u8,
        batch_id: String,
        cancelled: bool,
        files: Vec<BatchFileOutcome>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum BatchProgress {
    Started {
        version: u8,
        batch_id: String,
        total: usize,
    },
    FileStarted {
        version: u8,
        batch_id: String,
        index: usize,
        total: usize,
        name: String,
    },
    SaveStage {
        version: u8,
        batch_id: String,
        index: usize,
        total: usize,
        name: String,
        progress: SaveProgress,
    },
    FileFinished {
        version: u8,
        batch_id: String,
        outcome: BatchFileOutcome,
        total: usize,
    },
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelBatchRequest {
    version: u8,
    batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum CancelBatchResponse {
    Accepted { version: u8 },
    NotRunning { version: u8 },
    Error { version: u8 },
}

#[tauri::command]
pub(super) async fn import_fit_files(
    app: tauri::AppHandle,
    on_progress: Channel<BatchProgress>,
) -> BatchResponse {
    let (operation, started) = app.state::<Diagnostics>().begin(Event::BatchImportStarted);
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let diagnostics = app.state::<Diagnostics>();
        let mut batch_random = [0_u8; 16];
        if getrandom::fill(&mut batch_random).is_err() {
            return BatchResponse::Error {
                version: BATCH_VERSION,
                code: BatchCommandError::RandomnessUnavailable,
            };
        }
        let batch_id = batch_random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_state = app.state::<BatchCancellation>();
        let Ok(mut active) = cancel_state.0.try_lock() else {
            return BatchResponse::Error {
                version: BATCH_VERSION,
                code: BatchCommandError::LibraryBusy,
            };
        };
        if active.is_some() {
            return BatchResponse::Error {
                version: BATCH_VERSION,
                code: BatchCommandError::LibraryBusy,
            };
        }
        *active = Some(ActiveBatch {
            id: batch_id.clone(),
            cancelled: cancelled.clone(),
        });
        drop(active);
        let _active_guard = ActiveBatchGuard {
            state: &cancel_state,
            id: batch_id.clone(),
        };

        let paths = match app
            .dialog()
            .file()
            .add_filter("FIT activity", &["fit"])
            .blocking_pick_files()
        {
            None => {
                return BatchResponse::PickerCancelled {
                    version: BATCH_VERSION,
                }
            }
            Some(files) => files,
        };
        if paths.is_empty() {
            return BatchResponse::PickerCancelled {
                version: BATCH_VERSION,
            };
        }
        if paths.len() > MAX_BATCH_FILES {
            return BatchResponse::Error {
                version: BATCH_VERSION,
                code: BatchCommandError::TooManyFiles,
            };
        }
        let selected = paths
            .into_iter()
            .enumerate()
            .map(|(offset, file)| match file.into_path() {
                Ok(path) => SelectedFit {
                    name: display_name(&path, offset + 1),
                    path,
                },
                Err(_) => SelectedFit {
                    name: format!("Selected file {}", offset + 1),
                    path: PathBuf::new(),
                },
            })
            .collect::<Vec<_>>();

        let directory = match app.path().app_local_data_dir() {
            Ok(path) => path.join("library"),
            Err(_) => {
                return BatchResponse::Error {
                    version: BATCH_VERSION,
                    code: BatchCommandError::LibraryLocationUnavailable,
                };
            }
        };
        let provider = KeychainSecret {
            directory: &directory,
            service: "com.danmurphy.effortline.library.v1",
            account: "primary",
        };
        let library_state = app.state::<LibraryState>();
        let Ok(mut session) = library_state.0.try_lock() else {
            return BatchResponse::Error {
                version: BATCH_VERSION,
                code: BatchCommandError::LibraryBusy,
            };
        };
        if session.is_some() {
            diagnostics.record(operation, Event::LibraryReused);
        }
        let total = selected.len();
        let _ = on_progress.send(BatchProgress::Started {
            version: BATCH_VERSION,
            batch_id: batch_id.clone(),
            total,
        });
        let files = process_selected_files(
            &selected,
            &batch_id,
            &mut session,
            &directory,
            &provider,
            &cancelled,
            &mut |event| {
                match &event {
                    BatchProgress::FileFinished { outcome, .. } => {
                        diagnostics.record(
                            operation,
                            Event::BatchFileFinished {
                                status: outcome.status,
                                code: outcome.code,
                            },
                        );
                    }
                    BatchProgress::SaveStage { progress, .. } => {
                        diagnostics.record(
                            operation,
                            Event::BatchSaveProgress {
                                progress: progress.clone(),
                            },
                        );
                    }
                    _ => (),
                }
                let _ = on_progress.send(event);
            },
        );
        let was_cancelled = cancelled.load(Ordering::Relaxed);
        BatchResponse::Completed {
            version: BATCH_VERSION,
            batch_id,
            cancelled: was_cancelled,
            files,
        }
    })
    .await
    .unwrap_or(BatchResponse::Error {
        version: BATCH_VERSION,
        code: BatchCommandError::PickerFailed,
    });
    let (total, cancelled, error) = match &result {
        BatchResponse::PickerCancelled { .. } => (0, true, None),
        BatchResponse::Error { code, .. } => (0, false, Some(*code)),
        BatchResponse::Completed {
            files, cancelled, ..
        } => (files.len(), *cancelled, None),
    };
    app.state::<Diagnostics>().record(
        operation,
        Event::BatchImportFinished {
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            total,
            cancelled,
            error,
        },
    );
    result
}

#[tauri::command]
pub(super) fn cancel_fit_import(
    state: tauri::State<'_, BatchCancellation>,
    request: CancelBatchRequest,
) -> CancelBatchResponse {
    if request.version != BATCH_VERSION {
        return CancelBatchResponse::Error {
            version: BATCH_VERSION,
        };
    }
    if request.batch_id.len() != 32
        || !request
            .batch_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return CancelBatchResponse::Error {
            version: BATCH_VERSION,
        };
    }
    let Ok(active) = state.0.lock() else {
        return CancelBatchResponse::Error {
            version: BATCH_VERSION,
        };
    };
    if let Some(batch) = active.as_ref().filter(|batch| batch.id == request.batch_id) {
        batch.cancelled.store(true, Ordering::Relaxed);
        CancelBatchResponse::Accepted {
            version: BATCH_VERSION,
        }
    } else {
        CancelBatchResponse::NotRunning {
            version: BATCH_VERSION,
        }
    }
}

fn clear_active_batch(state: &BatchCancellation, batch_id: &str) {
    if let Ok(mut active) = state.0.lock() {
        if active.as_ref().is_some_and(|batch| batch.id == batch_id) {
            *active = None;
        }
    }
}

fn display_name(path: &Path, index: usize) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("Selected file {index}"));
    let display_name: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect();
    if display_name.is_empty() {
        format!("Selected file {index}")
    } else {
        display_name
    }
}

fn read_selected(path: &Path) -> Result<Zeroizing<Vec<u8>>, SaveError> {
    let file = File::open(path).map_err(|_| SaveError::FileReadFailed)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_FIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SaveError::FileReadFailed)?;
    if bytes.len() > MAX_FIT_BYTES {
        return Err(SaveError::FitTooLarge);
    }
    Ok(bytes)
}

fn is_file_error(error: SaveError) -> bool {
    matches!(
        error,
        SaveError::FileReadFailed
            | SaveError::FitTooLarge
            | SaveError::FitTooManyRecords
            | SaveError::FitTooManyDefinitions
            | SaveError::FitTruncated
            | SaveError::FitCorrupt
            | SaveError::FitUnsupported
            | SaveError::FitNotActivity
    )
}

fn process_selected_files(
    selected: &[SelectedFit],
    batch_id: &str,
    session: &mut Option<effortline_core::library::ActivityLibrary>,
    directory: &Path,
    provider: &impl effortline_core::library::LibrarySecretProvider,
    cancelled: &AtomicBool,
    report: &mut impl FnMut(BatchProgress),
) -> Vec<BatchFileOutcome> {
    let total = selected.len();
    let mut outcomes = Vec::with_capacity(total);
    let mut stopped_by_library_error = None;
    for (offset, selected) in selected.iter().enumerate() {
        let index = offset + 1;
        let file_started = BatchProgress::FileStarted {
            version: BATCH_VERSION,
            batch_id: batch_id.to_owned(),
            index,
            total,
            name: selected.name.clone(),
        };
        if cancelled.load(Ordering::Relaxed) || stopped_by_library_error.is_some() {
            let outcome = BatchFileOutcome {
                index,
                name: selected.name.clone(),
                status: BatchFileStatus::NotImported,
                code: stopped_by_library_error,
                activity: None,
            };
            report(file_started);
            report(BatchProgress::FileFinished {
                version: BATCH_VERSION,
                batch_id: batch_id.to_owned(),
                outcome: outcome.clone(),
                total,
            });
            outcomes.push(outcome);
            continue;
        }
        report(file_started);
        let result = if selected.path.as_os_str().is_empty() {
            Err(SaveError::FileReadFailed)
        } else {
            read_selected(&selected.path).and_then(|bytes| {
                save_bytes_in_session(session, &bytes, directory, provider, &mut |progress| {
                    report(BatchProgress::SaveStage {
                        version: BATCH_VERSION,
                        batch_id: batch_id.to_owned(),
                        index,
                        total,
                        name: selected.name.clone(),
                        progress: progress.into(),
                    });
                })
            })
        };
        let (status, code, activity) = match result {
            Ok(result) => {
                let status = match result.status {
                    ImportStatus::Saved => BatchFileStatus::Saved,
                    ImportStatus::AlreadyPresent => BatchFileStatus::AlreadyPresent,
                };
                (status, None, Some(result.summary.into()))
            }
            Err(error) => {
                if !is_file_error(error) {
                    stopped_by_library_error = Some(error);
                }
                (BatchFileStatus::Failed, Some(error), None)
            }
        };
        let outcome = BatchFileOutcome {
            index,
            name: selected.name.clone(),
            status,
            code,
            activity,
        };
        report(BatchProgress::FileFinished {
            version: BATCH_VERSION,
            batch_id: batch_id.to_owned(),
            outcome: outcome.clone(),
            total,
        });
        outcomes.push(outcome);
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit_preview::tests::synthetic_fit;
    use crate::library_save::{synthetic_support, SaveError};
    use effortline_core::library::{LibrarySecret, LibrarySecretProvider, SecretUnavailable};
    use std::sync::atomic::AtomicBool;

    struct TestSecret;
    impl LibrarySecretProvider for TestSecret {
        fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
            Ok(LibrarySecret::from_bytes([11; 32]))
        }
    }

    fn selected(directory: &Path, files: &[(&str, &[u8])]) -> Vec<SelectedFit> {
        files
            .iter()
            .map(|(name, bytes)| {
                let path = directory.join(name);
                std::fs::write(&path, bytes).unwrap();
                SelectedFit {
                    name: (*name).into(),
                    path,
                }
            })
            .collect()
    }

    fn run(directory: &Path, files: &[SelectedFit]) -> Vec<BatchFileOutcome> {
        process_selected_files(
            files,
            "synthetic-batch",
            &mut None,
            &directory.join("library"),
            &TestSecret,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    #[test]
    fn batch_reports_saved_duplicate_and_partial_file_failure_then_retry() {
        let directory = tempfile::tempdir().unwrap();
        let first = synthetic_fit(true);
        let second = synthetic_fit(false);
        let files = selected(
            directory.path(),
            &[
                ("first.fit", &first),
                ("bad.fit", b"synthetic invalid FIT"),
                ("second.fit", &second),
            ],
        );
        let original_contents = files
            .iter()
            .map(|file| std::fs::read(&file.path).unwrap())
            .collect::<Vec<_>>();

        let mut progress_events = Vec::new();
        let first_run = process_selected_files(
            &files,
            "synthetic-batch",
            &mut None,
            &directory.path().join("library"),
            &TestSecret,
            &AtomicBool::new(false),
            &mut |event| progress_events.push(event),
        );
        assert_eq!(first_run[0].status, BatchFileStatus::Saved);
        let summary = first_run[0].activity.unwrap();
        assert_eq!(
            summary.sport,
            crate::library_save::BatchActivitySport::Running
        );
        assert_eq!(summary.sample_count, 1);
        assert_eq!(first_run[1].status, BatchFileStatus::Failed);
        assert!(first_run[1].activity.is_none());
        assert_eq!(first_run[1].code, Some(SaveError::FitUnsupported));
        assert_eq!(first_run[2].status, BatchFileStatus::Saved);
        assert_eq!(
            files
                .iter()
                .map(|file| std::fs::read(&file.path).unwrap())
                .collect::<Vec<_>>(),
            original_contents,
            "source FIT files remain unchanged"
        );
        assert_eq!(
            progress_events
                .iter()
                .filter(|event| matches!(
                    event,
                    BatchProgress::SaveStage {
                        progress: SaveProgress::Started {
                            stage: crate::library_save::SaveStage::OpenRecovery,
                            ..
                        },
                        ..
                    }
                ))
                .count(),
            1,
            "a malformed member must not discard the healthy session"
        );

        let retry = run(directory.path(), &files);
        assert_eq!(retry[0].status, BatchFileStatus::AlreadyPresent);
        assert_eq!(retry[0].activity.unwrap(), summary);
        assert_eq!(retry[1].status, BatchFileStatus::Failed);
        assert_eq!(retry[2].status, BatchFileStatus::AlreadyPresent);
        let library = effortline_core::library::ActivityLibrary::open(
            directory.path().join("library"),
            &TestSecret,
        )
        .unwrap();
        for (bytes, selected) in [(&first, &files[0]), (&second, &files[2])] {
            let activity = effortline_core::fit_import::import_fit_activity(bytes).unwrap();
            assert!(library
                .contains_verified_source_with_progress(&activity.source.identity, &mut |_| {})
                .unwrap());
            assert_eq!(
                library
                    .read_original_bytes(&activity.source.identity)
                    .unwrap()
                    .unwrap(),
                *bytes,
                "the encrypted object must preserve exact source bytes"
            );
            assert_eq!(std::fs::read(&selected.path).unwrap(), *bytes);
        }
    }

    #[test]
    fn batch_cancel_finishes_current_file_and_marks_remaining_not_imported() {
        let directory = tempfile::tempdir().unwrap();
        let first = synthetic_fit(true);
        let second = synthetic_fit(false);
        let files = selected(
            directory.path(),
            &[("first.fit", &first), ("second.fit", &second)],
        );
        let cancelled = AtomicBool::new(false);
        let outcomes = process_selected_files(
            &files,
            "synthetic-batch",
            &mut None,
            &directory.path().join("library"),
            &TestSecret,
            &cancelled,
            &mut |event| {
                if matches!(event, BatchProgress::FileFinished { .. }) {
                    cancelled.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(outcomes[0].status, BatchFileStatus::Saved);
        assert_eq!(outcomes[1].status, BatchFileStatus::NotImported);
        assert_eq!(outcomes[1].code, None);
    }

    #[test]
    fn large_batch_file_reports_sample_progress_before_its_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = synthetic_support::synthetic_fit(4, true, 16_705);
        let files = selected(directory.path(), &[("large.fit", &bytes)]);
        let mut progress = Vec::new();
        let outcomes = process_selected_files(
            &files,
            "synthetic-batch",
            &mut None,
            &directory.path().join("library"),
            &TestSecret,
            &AtomicBool::new(false),
            &mut |event| progress.push(event),
        );
        assert_eq!(outcomes[0].status, BatchFileStatus::Saved);
        let last_sample_progress = progress
            .iter()
            .rposition(|event| {
                matches!(
                    event,
                    BatchProgress::SaveStage {
                        progress: SaveProgress::SamplesWritten {
                            completed: 16_705,
                            ..
                        },
                        ..
                    }
                )
            })
            .expect("sample progress reaches the final sample count");
        let file_finished = progress
            .iter()
            .position(|event| matches!(event, BatchProgress::FileFinished { .. }))
            .unwrap();
        assert!(last_sample_progress < file_finished);
    }
}
