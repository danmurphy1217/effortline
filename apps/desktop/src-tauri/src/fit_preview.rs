use crate::diagnostics::{Diagnostics, Event};
use effortline_core::fit_import::{import_fit_activity, ImportError, Sport, MAX_FIT_BYTES};
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct PreviewState(pub Mutex<Option<PendingPreview>>);

pub(super) struct PendingPreview {
    pub id: String,
    pub bytes: Zeroizing<Vec<u8>>,
    pub identity: effortline_core::fit_import::SourceIdentity,
}

const PREVIEW_VERSION: u8 = 1;

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum PreviewResponse {
    Ready {
        version: u8,
        preview_id: String,
        preview: FitPreview,
    },
    Cancelled {
        version: u8,
    },
    Error {
        version: u8,
        code: PreviewError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PreviewError {
    PickerFailed,
    PreviewBusy,
    RandomnessUnavailable,
    FileUnavailable,
    FileReadFailed,
    FitTooLarge,
    FitTooManyRecords,
    FitTooManyDefinitions,
    FitTruncated,
    FitCorrupt,
    FitUnsupported,
    FitNotActivity,
}

impl From<ImportError> for PreviewError {
    fn from(error: ImportError) -> Self {
        match error {
            ImportError::TooLarge => Self::FitTooLarge,
            ImportError::TooManyRecords => Self::FitTooManyRecords,
            ImportError::TooManyDefinitions => Self::FitTooManyDefinitions,
            ImportError::Truncated => Self::FitTruncated,
            ImportError::Corrupt => Self::FitCorrupt,
            ImportError::Unsupported => Self::FitUnsupported,
            ImportError::NotActivity => Self::FitNotActivity,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PreviewSport {
    Running,
    Other,
    Unknown,
}

impl From<Sport> for PreviewSport {
    fn from(sport: Sport) -> Self {
        match sport {
            Sport::Running => Self::Running,
            Sport::Other => Self::Other,
            Sport::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct FitPreview {
    sport: PreviewSport,
    duration_seconds: u64,
    distance_m: Option<f64>,
    pub(super) sample_count: usize,
    heart_rate_samples: usize,
}

impl PreviewResponse {
    fn error(code: PreviewError) -> Self {
        Self::Error {
            version: PREVIEW_VERSION,
            code,
        }
    }
}

#[tauri::command]
pub(super) async fn preview_fit_activity(app: tauri::AppHandle) -> PreviewResponse {
    let diagnostics = app.state::<Diagnostics>();
    let (operation, started) = diagnostics.begin(Event::PreviewStarted);
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let state = app.state::<PreviewState>();
        let Ok(mut pending) = state.0.try_lock() else {
            return PreviewResponse::error(PreviewError::PreviewBusy);
        };
        *pending = None;
        let selected = app
            .dialog()
            .file()
            .add_filter("FIT activity", &["fit"])
            .blocking_pick_file();
        match selected {
            None => preview_selected_path(None, &mut pending),
            Some(file) => match file.into_path() {
                Ok(path) => preview_selected_path(Some(path), &mut pending),
                Err(_) => PreviewResponse::error(PreviewError::FileUnavailable),
            },
        }
    })
    .await
    .unwrap_or_else(|_| PreviewResponse::error(PreviewError::PickerFailed));
    diagnostics.record(
        operation,
        Event::PreviewFinished {
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            outcome: (&result).into(),
        },
    );
    result
}

pub(super) fn preview_selected_path(
    path: Option<PathBuf>,
    pending: &mut Option<PendingPreview>,
) -> PreviewResponse {
    *pending = None;
    let Some(path) = path else {
        return PreviewResponse::Cancelled {
            version: PREVIEW_VERSION,
        };
    };
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => return PreviewResponse::error(PreviewError::FileReadFailed),
    };
    preview_from_reader(file, pending)
}

pub(super) fn preview_from_reader(
    reader: impl Read,
    pending: &mut Option<PendingPreview>,
) -> PreviewResponse {
    *pending = None;
    let mut bytes = Zeroizing::new(Vec::new());
    if reader
        .take(MAX_FIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return PreviewResponse::error(PreviewError::FileReadFailed);
    }
    if bytes.len() > MAX_FIT_BYTES {
        return PreviewResponse::error(PreviewError::FitTooLarge);
    }
    let activity = match import_fit_activity(&bytes) {
        Ok(activity) => activity,
        Err(error) => return PreviewResponse::error(error.into()),
    };
    let mut random = [0_u8; 16];
    if getrandom::fill(&mut random).is_err() {
        return PreviewResponse::error(PreviewError::RandomnessUnavailable);
    }
    let preview_id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    *pending = Some(PendingPreview {
        id: preview_id.clone(),
        identity: activity.source.identity,
        bytes,
    });
    let data = activity.data;
    PreviewResponse::Ready {
        version: PREVIEW_VERSION,
        preview_id,
        preview: FitPreview {
            sport: data.sport.into(),
            duration_seconds: data.end_unix_ms.saturating_sub(data.start_unix_ms).max(0) as u64
                / 1000,
            distance_m: data.total_distance_m,
            sample_count: data.samples.len(),
            heart_rate_samples: data
                .samples
                .iter()
                .filter(|sample| sample.heart_rate_bpm.is_some())
                .count(),
        },
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{self, Cursor};

    fn preview_from_reader(reader: impl Read) -> PreviewResponse {
        super::preview_from_reader(reader, &mut None)
    }

    fn preview_selected_path(path: Option<PathBuf>) -> PreviewResponse {
        super::preview_selected_path(path, &mut None)
    }

    // Synthetic FIT bytes only. No device export is used in these tests.
    fn definition(local: u8, global: u16, fields: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut bytes = vec![0x40 | local, 0, 0];
        bytes.extend(global.to_le_bytes());
        bytes.push(fields.len() as u8);
        for &(number, size, base_type) in fields {
            bytes.extend([number, size, base_type]);
        }
        bytes
    }

    fn crc16(bytes: &[u8]) -> u16 {
        let mut crc = 0_u16;
        for byte in bytes {
            crc ^= u16::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xA001
                } else {
                    crc >> 1
                };
            }
        }
        crc
    }

    pub(crate) fn synthetic_fit(with_heart_rate: bool) -> Vec<u8> {
        const FIT_TIME: u32 = 1_068_934_400;
        let mut data = definition(0, 0, &[(0, 1, 0)]);
        data.extend([0, 4]); // file_id, activity type
        data.extend(definition(
            1,
            18,
            &[(2, 4, 0x86), (5, 1, 0), (9, 4, 0x86), (253, 4, 0x86)],
        ));
        data.push(1);
        data.extend(FIT_TIME.to_le_bytes());
        data.push(1); // running
        data.extend(100_000_u32.to_le_bytes()); // 1 km in 1/100 m
        data.extend((FIT_TIME + 2).to_le_bytes());
        let record_fields = if with_heart_rate {
            vec![(253, 4, 0x86), (3, 1, 0x02)]
        } else {
            vec![(253, 4, 0x86)]
        };
        data.extend(definition(2, 20, &record_fields));
        data.push(2);
        data.extend((FIT_TIME + 1).to_le_bytes());
        if with_heart_rate {
            data.push(140);
        }

        let mut bytes = vec![12, 0x20, 0, 0];
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(b".FIT");
        bytes.extend(data);
        bytes.extend(crc16(&bytes).to_le_bytes());
        bytes
    }

    #[test]
    fn returns_versioned_summary_without_source_bytes_or_path() {
        let response = preview_from_reader(Cursor::new(synthetic_fit(true)));
        let mut serialized = serde_json::to_value(response).unwrap();
        assert_eq!(serialized["preview_id"].as_str().unwrap().len(), 32);
        serialized.as_object_mut().unwrap().remove("preview_id");
        assert_eq!(
            serialized,
            json!({
                "status": "ready",
                "version": 1,
                "preview": {
                    "sport": "running",
                    "duration_seconds": 2,
                    "distance_m": 1000.0,
                    "sample_count": 1,
                    "heart_rate_samples": 1
                }
            })
        );

        let without_heart_rate = preview_from_reader(Cursor::new(synthetic_fit(false)));
        let PreviewResponse::Ready { preview, .. } = without_heart_rate else {
            panic!("synthetic activity should preview");
        };
        assert_eq!(preview.heart_rate_samples, 0);
    }

    #[test]
    fn reads_selected_synthetic_file_and_maps_open_failure() {
        let path = std::env::temp_dir().join(format!(
            "effortline-synthetic-preview-{}.fit",
            std::process::id()
        ));
        std::fs::write(&path, synthetic_fit(true)).unwrap();
        let response = preview_selected_path(Some(path.clone()));
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(response, PreviewResponse::Ready { .. }));
        assert_eq!(
            preview_selected_path(Some(path)),
            PreviewResponse::error(PreviewError::FileReadFailed)
        );
    }

    #[test]
    fn maps_cancel_read_failure_and_bounded_input() {
        assert_eq!(
            preview_selected_path(None),
            PreviewResponse::Cancelled { version: 1 }
        );

        struct FailedReader;
        impl Read for FailedReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("synthetic read failure"))
            }
        }
        assert_eq!(
            preview_from_reader(FailedReader),
            PreviewResponse::error(PreviewError::FileReadFailed)
        );
        assert_eq!(
            preview_from_reader(io::repeat(0).take(MAX_FIT_BYTES as u64 + 1)),
            PreviewResponse::error(PreviewError::FitTooLarge)
        );
    }

    #[test]
    fn maps_import_failures_to_stable_error_codes() {
        let mut corrupt = synthetic_fit(true);
        *corrupt.last_mut().unwrap() ^= 1;
        assert_eq!(
            preview_from_reader(Cursor::new(corrupt)),
            PreviewResponse::error(PreviewError::FitCorrupt)
        );
        assert_eq!(
            preview_from_reader(Cursor::new(Vec::<u8>::new())),
            PreviewResponse::error(PreviewError::FitTruncated)
        );

        for (error, code) in [
            (ImportError::TooLarge, "fit_too_large"),
            (ImportError::TooManyRecords, "fit_too_many_records"),
            (ImportError::TooManyDefinitions, "fit_too_many_definitions"),
            (ImportError::Truncated, "fit_truncated"),
            (ImportError::Corrupt, "fit_corrupt"),
            (ImportError::Unsupported, "fit_unsupported"),
            (ImportError::NotActivity, "fit_not_activity"),
        ] {
            let response = PreviewResponse::error(error.into());
            assert_eq!(serde_json::to_value(response).unwrap()["code"], code);
        }
    }
}
