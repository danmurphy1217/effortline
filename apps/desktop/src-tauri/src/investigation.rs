use crate::diagnostics::{Diagnostics, Event};
use crate::library_save::{LibraryState, SaveError};
use crate::library_secret::KeychainSecret;
#[cfg(target_os = "macos")]
use crate::local_model::LocalModelRuntime;
use crate::local_model::{
    validate_chat_decision, ChatDecisionRejection, EvidenceAlias, GeneratedExplanation,
    ModelChatDecision, ModelChatMessage, ModelChatPhase, ModelChatRequest, ModelChatRole,
    ModelRunEvidence, ReplyScope,
};
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

const MAX_CHAT_MESSAGE_CHARS: usize = 800;
const MAX_CHAT_HISTORY_MESSAGES: usize = 12;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TrainingChatRequest {
    version: u8,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TrainingChatReplyKind {
    General,
    Evidence,
    Clarification,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct TrainingChatReply {
    pub(super) text: String,
    pub(super) kind: TrainingChatReplyKind,
    pub(super) origin: TrainingChatReplyOrigin,
    pub(super) citations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TrainingChatReplyOrigin {
    Model,
    Rust,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum TrainingChatResponse {
    Turn {
        version: u8,
        reply: TrainingChatReply,
        evidence: Option<Box<InvestigationResponse>>,
    },
    Error {
        version: u8,
        code: TrainingChatError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub(super) enum TrainingChatError {
    InvalidRequest,
    Busy,
    ModelNotInstalled,
    ModelUnavailable,
    UnsafeModelResponse,
    Library(SaveError),
}

#[derive(Default)]
pub(super) struct TrainingChatState(std::sync::Mutex<TrainingChatSession>);

#[derive(Default)]
struct TrainingChatSession {
    messages: Vec<ModelChatMessage>,
    evidence: Option<ChatEvidence>,
}

impl TrainingChatSession {
    fn messages_for_pending_turn(&self, user_text: &str) -> Vec<ModelChatMessage> {
        let mut messages = self.messages.clone();
        messages.push(ModelChatMessage {
            role: ModelChatRole::User,
            text: user_text.to_owned(),
        });
        messages
    }

    fn commit_turn(&mut self, user_text: String, assistant_text: String) {
        if self.messages.len() >= MAX_CHAT_HISTORY_MESSAGES {
            self.messages.drain(..2);
        }
        self.messages.push(ModelChatMessage {
            role: ModelChatRole::User,
            text: user_text,
        });
        self.messages.push(ModelChatMessage {
            role: ModelChatRole::Assistant,
            text: assistant_text,
        });
    }
}

#[derive(Clone)]
pub(super) struct ChatEvidence {
    pub(super) response: InvestigationResponse,
    pub(super) model_input: ModelInput,
    pub(super) source_ids: Vec<String>,
}

impl TrainingChatState {
    pub(super) fn reset(&self) -> bool {
        let Ok(mut session) = self.0.lock() else {
            return false;
        };
        *session = TrainingChatSession::default();
        true
    }
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

fn make_chat_evidence(value: RunningInvestigation) -> Result<ChatEvidence, TrainingChatError> {
    let response = response(value);
    let model_input = match &response {
        InvestigationResponse::Compared {
            previous_runs,
            recent_runs,
            pace,
            heart_rate,
            device_history,
            ..
        } => {
            let previous_model_runs =
                alias_runs::<{ effortline_core::investigation::RUNS_PER_PERIOD }>(previous_runs, 0)
                    .ok_or(TrainingChatError::UnsafeModelResponse)?;
            let recent_model_runs =
                alias_runs::<{ effortline_core::investigation::RUNS_PER_PERIOD }>(
                    recent_runs,
                    effortline_core::investigation::RUNS_PER_PERIOD,
                )
                .ok_or(TrainingChatError::UnsafeModelResponse)?;
            ModelInput::Compared(Box::new(ModelComparedInput {
                previous_runs: previous_model_runs,
                recent_runs: recent_model_runs,
                pace: pace.clone(),
                heart_rate: heart_rate.clone(),
                device_history: *device_history,
            }))
        }
        InvestigationResponse::InsufficientData {
            eligible_runs,
            required_runs,
            reason,
            ..
        } => ModelInput::InsufficientData {
            eligible_runs: *eligible_runs,
            required_runs: *required_runs,
            reason: match reason {
                InsufficientDataReason::TooFewRuns => InsufficientDataReason::TooFewRuns,
                InsufficientDataReason::AmbiguousPeriodBoundary => {
                    InsufficientDataReason::AmbiguousPeriodBoundary
                }
            },
        },
        InvestigationResponse::Error { code, .. } => {
            return Err(TrainingChatError::Library(*code));
        }
    };
    let source_ids = match &response {
        InvestigationResponse::Compared {
            previous_runs,
            recent_runs,
            ..
        } => previous_runs
            .iter()
            .chain(recent_runs.iter())
            .map(|run| run.source_id.clone())
            .collect(),
        InvestigationResponse::InsufficientData { .. } => Vec::new(),
        InvestigationResponse::Error { .. } => Vec::new(),
    };
    Ok(ChatEvidence {
        response,
        model_input,
        source_ids,
    })
}

pub(super) fn validated_chat_decision(
    decision: ModelChatDecision,
    phase: ModelChatPhase,
    evidence: Option<&ChatEvidence>,
) -> Result<ModelChatDecision, TrainingChatError> {
    validate_chat_decision(&decision, phase, evidence.is_some())
        .map_err(|_: ChatDecisionRejection| TrainingChatError::UnsafeModelResponse)?;
    if let (
        ModelChatDecision::Reply {
            text,
            scope: ReplyScope::Evidence,
            ..
        },
        Some(evidence),
    ) = (&decision, evidence)
    {
        validate_evidence_reply(text, evidence)
            .map_err(|_| TrainingChatError::UnsafeModelResponse)?;
    }
    Ok(decision)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EvidenceReplyRejection {
    UnsupportedCause,
    MissingHypothesisMarker,
    PersonalOrMeasuredClaim,
    InsufficientData,
}

pub(super) fn validate_evidence_reply(
    text: &str,
    evidence: &ChatEvidence,
) -> Result<(), EvidenceReplyRejection> {
    let lower = text.to_ascii_lowercase();
    let unsupported_causal_claim = [
        "because",
        "caused by",
        "due to",
        "resulted from",
        "driven by",
        "explained by",
        "as a result of",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    if unsupported_causal_claim {
        return Err(EvidenceReplyRejection::UnsupportedCause);
    }

    let words: Vec<&str> = lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let personal_or_measured_terms = [
        "you", "your", "athlete", "faster", "slower", "quicker", "similar", "improved", "declined",
    ];
    let marked_as_hypothesis = ["possible", "possibly", "may", "might", "could", "can"]
        .iter()
        .any(|word| words.contains(word));
    if !marked_as_hypothesis {
        return Err(EvidenceReplyRejection::MissingHypothesisMarker);
    }
    if words
        .iter()
        .any(|word| personal_or_measured_terms.contains(word))
    {
        return Err(EvidenceReplyRejection::PersonalOrMeasuredClaim);
    }
    if matches!(
        evidence.response,
        InvestigationResponse::InsufficientData { .. }
    ) {
        return Err(EvidenceReplyRejection::InsufficientData);
    }
    Ok(())
}

fn reply_from_decision(
    decision: ModelChatDecision,
    evidence: Option<&ChatEvidence>,
) -> Result<(TrainingChatReply, Option<InvestigationResponse>), TrainingChatError> {
    match decision {
        ModelChatDecision::Reply { text, scope, .. } => {
            let (kind, display_evidence) = match scope {
                ReplyScope::General => (TrainingChatReplyKind::General, None),
                ReplyScope::Evidence => (
                    TrainingChatReplyKind::Evidence,
                    evidence.map(|evidence| evidence.response.clone()),
                ),
            };
            Ok((
                TrainingChatReply {
                    text,
                    kind,
                    origin: TrainingChatReplyOrigin::Model,
                    citations: if scope == ReplyScope::Evidence {
                        evidence
                            .map(|evidence| evidence.source_ids.clone())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    },
                },
                display_evidence,
            ))
        }
        ModelChatDecision::AskClarifyingQuestion { text } => Ok((
            TrainingChatReply {
                text,
                kind: TrainingChatReplyKind::Clarification,
                origin: TrainingChatReplyOrigin::Model,
                citations: Vec::new(),
            },
            None,
        )),
        ModelChatDecision::CallTool => Err(TrainingChatError::UnsafeModelResponse),
    }
}

pub(super) fn fallback_chat_reply(evidence: &ChatEvidence) -> TrainingChatReply {
    let text = match &evidence.response {
        InvestigationResponse::InsufficientData {
            eligible_runs,
            required_runs,
            reason,
            ..
        } => match reason {
            InsufficientDataReason::TooFewRuns => format!(
                "There are too few eligible saved runs to compare. This comparison needs {required_runs} runs and found {eligible_runs}."
            ),
            InsufficientDataReason::AmbiguousPeriodBoundary => {
                "The comparison boundary is unclear because runs share a start time. I cannot assign them to either period safely.".into()
            }
        },
        InvestigationResponse::Compared {
            pace,
            heart_rate,
            device_history,
            ..
        } => {
            let pace_summary = if pace.change_percent < -0.5 {
                "The recent median running pace is faster than the previous group."
            } else if pace.change_percent > 0.5 {
                "The recent median running pace is slower than the previous group."
            } else {
                "The recent median running pace is similar to the previous group."
            };
            let heart_rate_limit = match heart_rate {
                HeartRateResult::Available { .. } => "",
                HeartRateResult::InsufficientCoverage { .. } => {
                    " The heart-rate comparison is limited by missing samples."
                }
            };
            let device_limit = match device_history {
                DeviceHistory::Consistent => "",
                DeviceHistory::Mixed => " The FIT files report different devices.",
                DeviceHistory::Missing => " Device details are missing from some FIT files.",
                DeviceHistory::MixedOrMissing => {
                    " Device details are mixed or missing in the FIT files."
                }
            };
            format!(
                "{pace_summary}{heart_rate_limit}{device_limit} This pattern does not show its cause."
            )
        }
        InvestigationResponse::Error { .. } => {
            "Effortline could not check the saved training evidence.".into()
        }
    };
    TrainingChatReply {
        text,
        kind: TrainingChatReplyKind::Evidence,
        origin: TrainingChatReplyOrigin::Rust,
        citations: evidence.source_ids.clone(),
    }
}

fn fallback_chat_turn(
    session: &mut TrainingChatSession,
    user_message: String,
    evidence: ChatEvidence,
) -> TrainingChatResponse {
    let reply = fallback_chat_reply(&evidence);
    session.commit_turn(user_message, reply.text.clone());
    session.evidence = Some(evidence.clone());
    TrainingChatResponse::Turn {
        version: VERSION,
        reply,
        evidence: Some(Box::new(evidence.response)),
    }
}

#[tauri::command]
pub(super) async fn training_chat(
    app: tauri::AppHandle,
    request: TrainingChatRequest,
    progress: Channel<InvestigationProgress>,
) -> TrainingChatResponse {
    if request.version != VERSION
        || request.message.trim().is_empty()
        || request.message.chars().count() > MAX_CHAT_MESSAGE_CHARS
    {
        return TrainingChatResponse::Error {
            version: VERSION,
            code: TrainingChatError::InvalidRequest,
        };
    }
    let (operation, started) = app
        .state::<Diagnostics>()
        .begin(Event::InvestigationStarted);
    let worker_app = app.clone();
    let message = request.message.trim().to_owned();
    match tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let chat_state = app.state::<TrainingChatState>();
        let Ok(mut session) = chat_state.0.try_lock() else {
            return TrainingChatResponse::Error {
                version: VERSION,
                code: TrainingChatError::Busy,
            };
        };
        let directory = match app.path().app_local_data_dir() {
            Ok(path) => path,
            Err(_) => {
                return TrainingChatResponse::Error {
                    version: VERSION,
                    code: TrainingChatError::ModelUnavailable,
                }
            }
        };
        let model_path = directory
            .join("models")
            .join(crate::local_model::MODEL_FILE_NAME);
        if !model_path.exists() {
            return TrainingChatResponse::Error {
                version: VERSION,
                code: TrainingChatError::ModelNotInstalled,
            };
        }
        let model_state = app.state::<crate::local_model::LocalModelState>();
        let Ok(_model_guard) = model_state.operation.try_lock() else {
            return TrainingChatResponse::Error {
                version: VERSION,
                code: TrainingChatError::Busy,
            };
        };
        if !model_state.is_verified(&model_path) {
            return TrainingChatResponse::Error {
                version: VERSION,
                code: TrainingChatError::ModelUnavailable,
            };
        }
        let mut on_stage = |stage| {
            let stage = match stage {
                crate::local_model::LocalModelStage::Loading => InvestigationStage::LoadingModel,
                crate::local_model::LocalModelStage::Generating
                | crate::local_model::LocalModelStage::ParsingOutput
                | crate::local_model::LocalModelStage::OutputMalformedJson => {
                    InvestigationStage::GeneratingExplanation
                }
            };
            report_stage(&app, operation, started, &progress, stage);
        };
        let initial_request = ModelChatRequest {
            phase: ModelChatPhase::ChooseAction,
            messages: session.messages_for_pending_turn(&message),
            previous_evidence: session
                .evidence
                .as_ref()
                .map(|evidence| evidence.model_input.clone()),
            tool_result: None,
        };
        let decision =
            match crate::local_model::run_chat_turn(&model_path, &initial_request, &mut on_stage) {
                Ok(decision) => decision,
                Err(_) => {
                    if let Some(evidence) = session.evidence.clone() {
                        return fallback_chat_turn(&mut session, message, evidence);
                    }
                    return TrainingChatResponse::Error {
                        version: VERSION,
                        code: TrainingChatError::ModelUnavailable,
                    };
                }
            };
        drop(_model_guard);
        let decision = match validated_chat_decision(
            decision,
            ModelChatPhase::ChooseAction,
            session.evidence.as_ref(),
        ) {
            Ok(decision) => decision,
            Err(code) => {
                if let Some(evidence) = session.evidence.clone() {
                    return fallback_chat_turn(&mut session, message, evidence);
                }
                return TrainingChatResponse::Error {
                    version: VERSION,
                    code,
                };
            }
        };
        let (decision, evidence) = match decision {
            ModelChatDecision::CallTool => {
                let library_state = app.state::<LibraryState>();
                let Ok(mut library_session) = library_state.0.try_lock() else {
                    return TrainingChatResponse::Error {
                        version: VERSION,
                        code: TrainingChatError::Busy,
                    };
                };
                let library_directory = directory.join("library");
                if library_session.is_none() {
                    report_stage(
                        &app,
                        operation,
                        started,
                        &progress,
                        InvestigationStage::OpeningLibrary,
                    );
                    match library_exists(&library_directory) {
                        Ok(false) => {}
                        Ok(true) => {
                            let provider = KeychainSecret {
                                directory: &library_directory,
                                service: "com.danmurphy.effortline.library.v1",
                                account: "primary",
                            };
                            match ActivityLibrary::open(&library_directory, &provider) {
                                Ok(library) => *library_session = Some(library),
                                Err(error) => {
                                    return TrainingChatResponse::Error {
                                        version: VERSION,
                                        code: TrainingChatError::Library(error.into()),
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            return TrainingChatResponse::Error {
                                version: VERSION,
                                code: TrainingChatError::Library(error),
                            }
                        }
                    }
                }
                report_stage(
                    &app,
                    operation,
                    started,
                    &progress,
                    InvestigationStage::AnalyzingActivities,
                );
                let result = if let Some(library) = library_session.as_ref() {
                    match analyze_recent_running(library) {
                        Ok(result) => result,
                        Err(error) => {
                            return TrainingChatResponse::Error {
                                version: VERSION,
                                code: TrainingChatError::Library(error.into()),
                            }
                        }
                    }
                } else {
                    RunningInvestigation::InsufficientData {
                        eligible_runs: 0,
                        required_runs: 6,
                        reason: CoreInsufficientDataReason::TooFewRuns,
                    }
                };
                let evidence = match make_chat_evidence(result) {
                    Ok(evidence) => evidence,
                    Err(code) => {
                        return TrainingChatResponse::Error {
                            version: VERSION,
                            code,
                        }
                    }
                };
                if matches!(&evidence.model_input, ModelInput::InsufficientData { .. }) {
                    return fallback_chat_turn(&mut session, message, evidence);
                }
                drop(library_session);
                let Ok(_final_model_guard) = model_state.operation.try_lock() else {
                    return TrainingChatResponse::Error {
                        version: VERSION,
                        code: TrainingChatError::Busy,
                    };
                };
                if !model_state.is_verified(&model_path) {
                    return fallback_chat_turn(&mut session, message, evidence);
                }
                let final_request = ModelChatRequest {
                    phase: ModelChatPhase::RespondFromTool,
                    messages: session.messages_for_pending_turn(&message),
                    previous_evidence: None,
                    tool_result: Some(evidence.model_input.clone()),
                };
                let final_decision = match crate::local_model::run_chat_turn(
                    &model_path,
                    &final_request,
                    &mut on_stage,
                ) {
                    Ok(decision) => decision,
                    Err(_) => return fallback_chat_turn(&mut session, message, evidence),
                };
                let final_decision = match validated_chat_decision(
                    final_decision,
                    ModelChatPhase::RespondFromTool,
                    Some(&evidence),
                ) {
                    Ok(decision) => decision,
                    Err(_) => return fallback_chat_turn(&mut session, message, evidence),
                };
                session.evidence = Some(evidence.clone());
                (final_decision, Some(evidence))
            }
            decision => (decision, session.evidence.clone()),
        };
        let allowed_evidence = evidence.as_ref();
        let (reply, display_evidence) = match reply_from_decision(decision, allowed_evidence) {
            Ok(result) => result,
            Err(_) => match evidence {
                Some(evidence) => return fallback_chat_turn(&mut session, message, evidence),
                None => {
                    return TrainingChatResponse::Error {
                        version: VERSION,
                        code: TrainingChatError::UnsafeModelResponse,
                    }
                }
            },
        };
        session.commit_turn(message, reply.text.clone());
        TrainingChatResponse::Turn {
            version: VERSION,
            reply,
            evidence: display_evidence.map(Box::new),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => TrainingChatResponse::Error {
            version: VERSION,
            code: TrainingChatError::ModelUnavailable,
        },
    }
}

#[tauri::command]
pub(super) fn reset_training_chat(state: tauri::State<'_, TrainingChatState>) -> bool {
    state.reset()
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
    use crate::local_model::ModelCitation;

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

    #[test]
    fn safe_chat_fallback_uses_only_tool_facts_and_keeps_citations() {
        let response = InvestigationResponse::Compared {
            version: VERSION,
            previous_runs: Vec::new(),
            recent_runs: Vec::new(),
            pace: PaceResult {
                previous_median_seconds_per_km: 300.0,
                recent_median_seconds_per_km: 280.0,
                change_percent: -6.6,
            },
            heart_rate: HeartRateResult::InsufficientCoverage {
                previous_qualified_runs: 2,
                recent_qualified_runs: 1,
                required_runs_per_period: 3,
                minimum_samples_per_run: 10,
                minimum_coverage_percent: 50,
            },
            device_history: DeviceHistory::MixedOrMissing,
            explanation: None,
            explanation_error: None,
        };
        let model_input = ModelInput::Compared(Box::new(ModelComparedInput {
            previous_runs: std::array::from_fn(run_evidence),
            recent_runs: std::array::from_fn(|index| run_evidence(index + 3)),
            pace: PaceResult {
                previous_median_seconds_per_km: 300.0,
                recent_median_seconds_per_km: 280.0,
                change_percent: -6.6,
            },
            heart_rate: HeartRateResult::InsufficientCoverage {
                previous_qualified_runs: 2,
                recent_qualified_runs: 1,
                required_runs_per_period: 3,
                minimum_samples_per_run: 10,
                minimum_coverage_percent: 50,
            },
            device_history: DeviceHistory::MixedOrMissing,
        }));
        let evidence = ChatEvidence {
            response,
            model_input,
            source_ids: (0..6)
                .map(|index| format!("synthetic-source-{index}"))
                .collect(),
        };
        let reply = fallback_chat_reply(&evidence);

        assert_eq!(reply.kind, TrainingChatReplyKind::Evidence);
        assert_eq!(reply.origin, TrainingChatReplyOrigin::Rust);
        assert_eq!(reply.citations.len(), 6);
        assert!(reply.text.contains("faster"));
        assert!(reply.text.contains("heart-rate comparison is limited"));
        assert!(reply.text.contains("Device details are mixed or missing"));
        assert!(reply.text.contains("does not show its cause"));
    }

    #[test]
    fn sparse_chat_fallback_reports_the_tool_limit_without_citations() {
        let evidence = ChatEvidence {
            response: InvestigationResponse::InsufficientData {
                version: VERSION,
                eligible_runs: 2,
                required_runs: 6,
                reason: InsufficientDataReason::TooFewRuns,
            },
            model_input: ModelInput::InsufficientData {
                eligible_runs: 2,
                required_runs: 6,
                reason: InsufficientDataReason::TooFewRuns,
            },
            source_ids: Vec::new(),
        };
        let reply = fallback_chat_reply(&evidence);

        assert_eq!(reply.citations, Vec::<String>::new());
        assert!(reply.text.contains("needs 6 runs and found 2"));
    }

    #[test]
    fn failed_turn_is_not_added_to_chat_history_until_a_reply_is_ready() {
        let mut session = TrainingChatSession::default();
        let pending = session.messages_for_pending_turn("Can you compare my recent runs?");

        assert_eq!(pending.len(), 1);
        assert!(session.messages.is_empty());

        session.commit_turn(
            "Can you compare my recent runs?".into(),
            "The recent pace was faster.".into(),
        );
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, ModelChatRole::User);
        assert_eq!(session.messages[1].role, ModelChatRole::Assistant);

        let follow_up = session.messages_for_pending_turn("What might explain that?");
        assert_eq!(follow_up.len(), 3);
        assert_eq!(follow_up[0].role, ModelChatRole::User);
        assert_eq!(follow_up[1].role, ModelChatRole::Assistant);
        assert_eq!(follow_up[2].role, ModelChatRole::User);
    }

    #[test]
    fn evidence_reply_rejects_opposite_pace_and_unproven_cause() {
        let evidence = ChatEvidence {
            response: InvestigationResponse::Compared {
                version: VERSION,
                previous_runs: Vec::new(),
                recent_runs: Vec::new(),
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
                explanation: None,
                explanation_error: None,
            },
            model_input: ModelInput::Compared(Box::new(ModelComparedInput {
                previous_runs: std::array::from_fn(run_evidence),
                recent_runs: std::array::from_fn(|index| run_evidence(index + 3)),
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
            })),
            source_ids: vec!["synthetic-source".into()],
        };

        let decision = |text: &str| ModelChatDecision::Reply {
            text: text.into(),
            scope: ReplyScope::Evidence,
            citations: vec![ModelCitation("E99".into())],
        };
        assert_eq!(
            validated_chat_decision(
                decision("Your recent pace improved."),
                ModelChatPhase::ChooseAction,
                Some(&evidence),
            ),
            Err(TrainingChatError::UnsafeModelResponse)
        );
        assert_eq!(
            validated_chat_decision(
                decision("A change in routine helped your progress."),
                ModelChatPhase::ChooseAction,
                Some(&evidence),
            ),
            Err(TrainingChatError::UnsafeModelResponse)
        );
        assert_eq!(
            validated_chat_decision(
                decision("A possible factor to consider is recovery, which may affect pace."),
                ModelChatPhase::ChooseAction,
                Some(&evidence),
            )
            .unwrap(),
            decision("A possible factor to consider is recovery, which may affect pace.")
        );
        let accepted = validated_chat_decision(
            decision("A possible factor to consider is recovery, which may affect pace."),
            ModelChatPhase::ChooseAction,
            Some(&evidence),
        )
        .expect("the general hypothesis is allowed");
        let (reply, _) = reply_from_decision(accepted, Some(&evidence))
            .expect("Rust attaches citations to the evidence it supplied");
        assert_eq!(reply.citations, evidence.source_ids);
    }

    #[test]
    fn general_reply_has_no_model_supplied_citations() {
        let decision = ModelChatDecision::Reply {
            text: "A steady routine can support endurance.".into(),
            scope: ReplyScope::General,
            citations: Vec::new(),
        };
        let expected = decision.clone();

        let validated = validated_chat_decision(decision, ModelChatPhase::ChooseAction, None)
            .expect("general replies do not cite athlete evidence");
        assert_eq!(validated, expected);
    }

    fn run_evidence(index: usize) -> ModelRunEvidence {
        ModelRunEvidence {
            source_id: EvidenceAlias::from_index(index).unwrap(),
            started_at_unix_ms: 1_700_000_000_000,
            duration_seconds: 300,
            distance_m: 1_000.0,
            pace_seconds_per_km: 300.0,
            sample_count: 20,
            heart_rate_sample_count: 20,
            median_heart_rate_bpm: Some(140.0),
        }
    }
}
