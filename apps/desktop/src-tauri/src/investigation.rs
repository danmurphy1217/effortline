use crate::diagnostics::{Diagnostics, Event};
use crate::library_save::{LibraryState, SaveError};
use crate::library_secret::KeychainSecret;
#[cfg(target_os = "macos")]
use crate::local_model::LocalModelRuntime;
use crate::local_model::{EvidenceAlias, GeneratedExplanation, ModelRunEvidence};
use effortline_core::investigation::{
    investigate_recent_running as analyze_recent_running, ActivityEvidence,
    DeviceHistory as CoreDeviceHistory, HeartRateComparison,
    InsufficientDataReason as CoreInsufficientDataReason, RunningInvestigation,
};
use effortline_core::library::ActivityLibrary;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;
use tauri::ipc::Channel;
use tauri::Manager;

const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InvestigationStage {
    OpeningLibrary,
    AnalyzingActivities,
    VerifyingModel,
    LoadingModel,
    GeneratingExplanation,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(super) struct InvestigationProgress {
    version: u8,
    stage: InvestigationStage,
    elapsed_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InvestigationOutcome {
    Compared,
    InsufficientData,
    Error,
}

fn report_stage(
    app: &tauri::AppHandle,
    operation: u64,
    started: Instant,
    progress: &Channel<InvestigationProgress>,
    stage: InvestigationStage,
) {
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let update = InvestigationProgress {
        version: VERSION,
        stage,
        elapsed_ms,
    };
    let _ = progress.send(update);
    app.state::<Diagnostics>()
        .record(operation, Event::InvestigationStage { stage, elapsed_ms });
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InvestigationRequest {
    version: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct RunningEvidence {
    pub(super) source_id: String,
    pub(super) started_at_unix_ms: i64,
    pub(super) duration_seconds: u64,
    pub(super) distance_m: f64,
    pub(super) pace_seconds_per_km: f64,
    pub(super) sample_count: usize,
    pub(super) heart_rate_sample_count: usize,
    pub(super) median_heart_rate_bpm: Option<f64>,
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
        device_history: DeviceHistory,
        explanation: Option<GeneratedExplanation>,
        explanation_error: Option<crate::local_model::LocalModelError>,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum ModelInput {
    Compared(Box<ModelComparedInput>),
    // Sparse results stay deterministic in the live flow; this variant supports the synthetic
    // model evaluation contract and is not sent to inference for real insufficient-data results.
    #[allow(dead_code)]
    InsufficientData {
        eligible_runs: usize,
        required_runs: usize,
        reason: InsufficientDataReason,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct ModelComparedInput {
    pub(super) previous_runs: [ModelRunEvidence; effortline_core::investigation::RUNS_PER_PERIOD],
    pub(super) recent_runs: [ModelRunEvidence; effortline_core::investigation::RUNS_PER_PERIOD],
    pub(super) pace: PaceResult,
    pub(super) heart_rate: HeartRateResult,
    pub(super) device_history: DeviceHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InsufficientDataReason {
    TooFewRuns,
    AmbiguousPeriodBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DeviceHistory {
    Consistent,
    Mixed,
    Missing,
    MixedOrMissing,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct PaceResult {
    pub(super) previous_median_seconds_per_km: f64,
    pub(super) recent_median_seconds_per_km: f64,
    pub(super) change_percent: f64,
}

fn response(value: RunningInvestigation) -> InvestigationResponse {
    match value {
        RunningInvestigation::Compared {
            previous_runs,
            recent_runs,
            pace,
            heart_rate,
            device_history,
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
            device_history: match device_history {
                CoreDeviceHistory::Consistent => DeviceHistory::Consistent,
                CoreDeviceHistory::Mixed => DeviceHistory::Mixed,
                CoreDeviceHistory::Missing => DeviceHistory::Missing,
                CoreDeviceHistory::MixedOrMissing => DeviceHistory::MixedOrMissing,
            },
            explanation: None,
            explanation_error: None,
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

fn alias_runs<const N: usize>(
    runs: &[RunningEvidence],
    alias_offset: usize,
) -> Option<[ModelRunEvidence; N]> {
    if runs.len() != N {
        return None;
    }
    let evidence = runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            Some(ModelRunEvidence {
                source_id: EvidenceAlias::from_index(alias_offset + index)?,
                started_at_unix_ms: run.started_at_unix_ms,
                duration_seconds: run.duration_seconds,
                distance_m: run.distance_m,
                pace_seconds_per_km: run.pace_seconds_per_km,
                sample_count: run.sample_count,
                heart_rate_sample_count: run.heart_rate_sample_count,
                median_heart_rate_bpm: run.median_heart_rate_bpm,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    evidence.try_into().ok()
}

#[tauri::command]
pub(super) async fn investigate_recent_running(
    app: tauri::AppHandle,
    request: InvestigationRequest,
    progress: Channel<InvestigationProgress>,
) -> InvestigationResponse {
    if request.version != VERSION {
        return InvestigationResponse::Error {
            version: VERSION,
            code: SaveError::UnsupportedRequest,
        };
    }
    let (diagnostic_operation, started) = app
        .state::<Diagnostics>()
        .begin(Event::InvestigationStarted);
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
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
            report_stage(
                &app,
                diagnostic_operation,
                started,
                &progress,
                InvestigationStage::OpeningLibrary,
            );
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
        report_stage(
            &app,
            diagnostic_operation,
            started,
            &progress,
            InvestigationStage::AnalyzingActivities,
        );
        match analyze_recent_running(library) {
            Ok(result) => {
                let mut answer = response(result);
                #[cfg(target_os = "macos")]
                let (generated, model_error) = if let InvestigationResponse::Compared {
                    previous_runs,
                    recent_runs,
                    pace,
                    heart_rate,
                    device_history,
                    ..
                } = &answer
                {
                    let ids: Vec<_> = previous_runs
                        .iter()
                        .chain(recent_runs.iter())
                        .map(|run| run.source_id.clone())
                        .collect();
                    let heart_rate_available =
                        matches!(heart_rate, HeartRateResult::Available { .. });
                    if let (Some(previous_model_runs), Some(recent_model_runs)) = (
                        alias_runs::<{ effortline_core::investigation::RUNS_PER_PERIOD }>(
                            previous_runs,
                            0,
                        ),
                        alias_runs::<{ effortline_core::investigation::RUNS_PER_PERIOD }>(
                            recent_runs,
                            effortline_core::investigation::RUNS_PER_PERIOD,
                        ),
                    ) {
                        let aliases: Vec<_> = previous_model_runs
                            .iter()
                            .chain(&recent_model_runs)
                            .map(|run| run.source_id)
                            .collect();
                        let bounded = ModelInput::Compared(Box::new(ModelComparedInput {
                            previous_runs: previous_model_runs,
                            recent_runs: recent_model_runs,
                            pace: pace.clone(),
                            heart_rate: heart_rate.clone(),
                            device_history: *device_history,
                        }));
                        if let Ok(path) = app.path().app_local_data_dir().map(|path| {
                        path.join("models")
                            .join(crate::local_model::MODEL_FILE_NAME)
                    }) {
                        if path.exists() {
                            report_stage(
                                &app,
                                diagnostic_operation,
                                started,
                                &progress,
                                InvestigationStage::VerifyingModel,
                            );
                        }
                        let operation = app
                            .state::<crate::local_model::LocalModelState>()
                            .operation
                            .clone();
                        let result = if let Ok(_guard) = operation.try_lock() {
                            if !path.exists() {
                                (None, None)
                            } else if crate::local_model::verify_installed_model(&path) {
                                let mut report_model_stage = |stage| {
                                    let stage = match stage {
                                        crate::local_model::LocalModelStage::Loading => {
                                            InvestigationStage::LoadingModel
                                        }
                                        crate::local_model::LocalModelStage::Generating => {
                                            InvestigationStage::GeneratingExplanation
                                        }
                                        crate::local_model::LocalModelStage::ParsingOutput => {
                                            InvestigationStage::GeneratingExplanation
                                        }
                                        crate::local_model::LocalModelStage::OutputMalformedJson => {
                                            InvestigationStage::GeneratingExplanation
                                        }
                                    };
                                    report_stage(
                                        &app,
                                        diagnostic_operation,
                                        started,
                                        &progress,
                                        stage,
                                    );
                                };
                                match crate::local_model::LlamaCppRuntime.explain(
                                    &path,
                                    &bounded,
                                    &mut report_model_stage,
                                ) {
                                    Ok(output) => match crate::local_model::validate_explanation(
                                        &output,
                                        &aliases,
                                        heart_rate_available,
                                    ) {
                                        Ok(valid) => {
                                            let citations = valid
                                                .citations
                                                .iter()
                                                .map(|alias| ids.get(alias.index()).cloned())
                                                .collect::<Option<Vec<_>>>();
                                            if let Some(citations) = citations {
                                                (Some(GeneratedExplanation {
                                                    text: valid.text,
                                                    citations,
                                                }), None)
                                            } else {
                                                (
                                                    None,
                                                    Some(crate::local_model::LocalModelError::InferenceFailed),
                                                )
                                            }
                                        }
                                        Err(error) => (None, Some(error)),
                                    },
                                    Err(error) => (None, Some(error)),
                                }
                            } else {
                                (
                                    None,
                                    Some(crate::local_model::LocalModelError::VerificationFailed),
                                )
                            }
                        } else {
                            (
                                None,
                                Some(crate::local_model::LocalModelError::InstallInProgress),
                            )
                        };
                        result
                    } else {
                        (
                            None,
                            Some(crate::local_model::LocalModelError::LocationUnavailable),
                        )
                    }
                    } else {
                        (
                            None,
                            Some(crate::local_model::LocalModelError::InferenceFailed),
                        )
                    }
                } else {
                    (None, None)
                };
                if let InvestigationResponse::Compared {
                    explanation,
                    explanation_error,
                    ..
                } = &mut answer
                {
                    #[cfg(target_os = "macos")]
                    {
                        *explanation = generated;
                        *explanation_error = model_error;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        *explanation = None;
                        *explanation_error = None;
                    }
                }
                answer
            }
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
    });
    let outcome = match &result {
        InvestigationResponse::Compared { .. } => InvestigationOutcome::Compared,
        InvestigationResponse::InsufficientData { .. } => InvestigationOutcome::InsufficientData,
        InvestigationResponse::Error { .. } => InvestigationOutcome::Error,
    };
    app.state::<Diagnostics>().record(
        diagnostic_operation,
        Event::InvestigationFinished {
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            outcome,
        },
    );
    result
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
    fn typed_model_input_uses_aliases_instead_of_source_hashes() {
        let source_id = "private-source-hash";
        let run = RunningEvidence {
            source_id: source_id.into(),
            started_at_unix_ms: 1_700_000_000_000,
            duration_seconds: 300,
            distance_m: 1_000.0,
            pace_seconds_per_km: 300.0,
            sample_count: 20,
            heart_rate_sample_count: 20,
            median_heart_rate_bpm: Some(140.0),
        };
        let runs: [RunningEvidence; effortline_core::investigation::RUNS_PER_PERIOD] =
            std::array::from_fn(|_| run.clone());
        let previous_runs = alias_runs(&runs, 0).unwrap();
        let recent_runs =
            alias_runs(&runs, effortline_core::investigation::RUNS_PER_PERIOD).unwrap();
        let input = ModelInput::Compared(Box::new(ModelComparedInput {
            previous_runs,
            recent_runs,
            pace: PaceResult {
                previous_median_seconds_per_km: 300.0,
                recent_median_seconds_per_km: 280.0,
                change_percent: -6.6,
            },
            heart_rate: HeartRateResult::Available {
                previous_median_bpm: 140.0,
                recent_median_bpm: 138.0,
            },
            device_history: DeviceHistory::Consistent,
        }));
        let serialized = serde_json::to_string(&input).unwrap();

        assert_eq!(run.source_id, source_id);
        assert!(serialized.contains("E1"));
        assert!(!serialized.contains(source_id));
    }

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
