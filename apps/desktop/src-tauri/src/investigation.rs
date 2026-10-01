use crate::library_save::{LibraryState, SaveError};
use crate::library_secret::KeychainSecret;
use effortline_core::investigation::{
    investigate_recent_running as analyze_recent_running, ActivityEvidence, HeartRateComparison,
    InsufficientDataReason as CoreInsufficientDataReason, RunningInvestigation,
};
use effortline_core::library::ActivityLibrary;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Manager;

const VERSION: u8 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InvestigationRequest {
    version: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct RunningEvidence {
    source_id: String,
    started_at_unix_ms: i64,
    duration_seconds: u64,
    distance_m: f64,
    pace_seconds_per_km: f64,
    sample_count: usize,
    heart_rate_sample_count: usize,
    median_heart_rate_bpm: Option<f64>,
}

impl From<ActivityEvidence> for RunningEvidence {
    fn from(evidence: ActivityEvidence) -> Self {
        Self {
            source_id: evidence.source_id,
            started_at_unix_ms: evidence.started_at_unix_ms,
            duration_seconds: evidence.duration_seconds,
            distance_m: evidence.distance_m,
            pace_seconds_per_km: evidence.pace_seconds_per_km,
            sample_count: evidence.sample_count,
            heart_rate_sample_count: evidence.heart_rate_sample_count,
            median_heart_rate_bpm: evidence.median_heart_rate_bpm,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum HeartRateResult {
    Available {
        previous_median_bpm: f64,
        recent_median_bpm: f64,
    },
    InsufficientCoverage {
        previous_qualified_runs: usize,
        recent_qualified_runs: usize,
        required_runs_per_period: usize,
        minimum_samples_per_run: usize,
        minimum_coverage_percent: u8,
    },
}

impl From<HeartRateComparison> for HeartRateResult {
    fn from(value: HeartRateComparison) -> Self {
        match value {
            HeartRateComparison::Available {
                previous_median_bpm,
                recent_median_bpm,
            } => Self::Available {
                previous_median_bpm,
                recent_median_bpm,
            },
            HeartRateComparison::InsufficientCoverage {
                previous_qualified_runs,
                recent_qualified_runs,
                required_runs_per_period,
                minimum_samples_per_run,
                minimum_coverage_percent,
            } => Self::InsufficientCoverage {
                previous_qualified_runs,
                recent_qualified_runs,
                required_runs_per_period,
                minimum_samples_per_run,
                minimum_coverage_percent,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum InvestigationResponse {
    Compared {
        version: u8,
        previous_runs: Vec<RunningEvidence>,
        recent_runs: Vec<RunningEvidence>,
        pace: PaceResult,
        heart_rate: HeartRateResult,
    },
    InsufficientData {
        version: u8,
        eligible_runs: usize,
        required_runs: usize,
        reason: InsufficientDataReason,
    },
    Error {
        version: u8,
        code: SaveError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InsufficientDataReason {
    TooFewRuns,
    AmbiguousPeriodBoundary,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct PaceResult {
    previous_median_seconds_per_km: f64,
    recent_median_seconds_per_km: f64,
    change_percent: f64,
}

fn response(value: RunningInvestigation) -> InvestigationResponse {
    match value {
        RunningInvestigation::Compared {
            previous_runs,
            recent_runs,
            pace,
            heart_rate,
        } => InvestigationResponse::Compared {
            version: VERSION,
            previous_runs: previous_runs.into_iter().map(Into::into).collect(),
            recent_runs: recent_runs.into_iter().map(Into::into).collect(),
            pace: PaceResult {
                previous_median_seconds_per_km: pace.previous_median_seconds_per_km,
                recent_median_seconds_per_km: pace.recent_median_seconds_per_km,
                change_percent: pace.change_percent,
            },
            heart_rate: heart_rate.into(),
        },
        RunningInvestigation::InsufficientData {
            eligible_runs,
            required_runs,
            reason,
        } => InvestigationResponse::InsufficientData {
            version: VERSION,
            eligible_runs,
            required_runs,
            reason: match reason {
                CoreInsufficientDataReason::TooFewRuns => InsufficientDataReason::TooFewRuns,
                CoreInsufficientDataReason::AmbiguousPeriodBoundary => {
                    InsufficientDataReason::AmbiguousPeriodBoundary
                }
            },
        },
    }
}

#[tauri::command]
pub(super) async fn investigate_recent_running(
    app: tauri::AppHandle,
    request: InvestigationRequest,
) -> InvestigationResponse {
    if request.version != VERSION {
        return InvestigationResponse::Error {
            version: VERSION,
            code: SaveError::UnsupportedRequest,
        };
    }
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let state = app.state::<LibraryState>();
        let Ok(mut session) = state.0.try_lock() else {
            return InvestigationResponse::Error {
                version: VERSION,
                code: SaveError::LibraryBusy,
            };
        };
        let directory = match app.path().app_local_data_dir() {
            Ok(path) => path.join("library"),
            Err(_) => {
                return InvestigationResponse::Error {
                    version: VERSION,
                    code: SaveError::LibraryLocationUnavailable,
                }
            }
        };
        if session.is_none() {
            match library_exists(&directory) {
                Ok(false) => {
                    return response(RunningInvestigation::InsufficientData {
                        eligible_runs: 0,
                        required_runs: 6,
                        reason: CoreInsufficientDataReason::TooFewRuns,
                    })
                }
                Ok(true) => {}
                Err(code) => {
                    return InvestigationResponse::Error {
                        version: VERSION,
                        code,
                    }
                }
            }
            let provider = KeychainSecret {
                directory: &directory,
                service: "com.danmurphy.effortline.library.v1",
                account: "primary",
            };
            match ActivityLibrary::open(&directory, &provider) {
                Ok(library) => *session = Some(library),
                Err(error) => {
                    return InvestigationResponse::Error {
                        version: VERSION,
                        code: error.into(),
                    }
                }
            }
        }
        let Some(library) = session.as_ref() else {
            return InvestigationResponse::Error {
                version: VERSION,
                code: SaveError::SaveFailed,
            };
        };
        match analyze_recent_running(library) {
            Ok(result) => response(result),
            Err(error) => InvestigationResponse::Error {
                version: VERSION,
                code: error.into(),
            },
        }
    })
    .await
    .unwrap_or(InvestigationResponse::Error {
        version: VERSION,
        code: SaveError::SaveFailed,
    })
}

fn library_exists(directory: &Path) -> Result<bool, SaveError> {
    match (
        directory.join("library.sqlite3").try_exists(),
        directory.join("objects").try_exists(),
    ) {
        (Ok(false), Ok(false)) => Ok(false),
        (Ok(true), Ok(true)) => Ok(true),
        (Ok(_), Ok(_)) => Err(SaveError::LibraryIncomplete),
        _ => Err(SaveError::LibraryIo),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data_response_is_versioned_and_typed() {
        let value = serde_json::to_value(response(RunningInvestigation::InsufficientData {
            eligible_runs: 2,
            required_runs: 6,
            reason: CoreInsufficientDataReason::TooFewRuns,
        }))
        .unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["status"], "insufficient_data");
        assert_eq!(value["eligible_runs"], 2);
        assert_eq!(value["required_runs"], 6);
        assert_eq!(value["reason"], "too_few_runs");
    }
}
