import { useEffect, useState, type FormEvent } from "react";
import {
  cancelLocalModelInstall,
  getLocalModelStatus,
  installLocalModel,
  modelErrorMessage,
  removeLocalModel,
  type LocalModelProgress,
  type LocalModelStatus,
} from "./localModel";
import {
  requestFitPreview,
  type PreviewErrorCode,
  type PreviewResponse,
} from "./fitPreview";
import {
  saveFitPreview,
  checkFitPreview,
  type LibraryCheckState,
  saveErrorMessage,
  type SaveResponse,
  type SaveProgress,
  saveProgressMessage,
  saveButtonState,
  importFitFiles,
  cancelFitImport,
  batchFileErrorMessage,
  batchCommandErrorMessage,
  type BatchActivitySummary,
  type BatchProgress,
  type BatchResponse,
  type BatchFileOutcome,
} from "./librarySave";
import {
  investigationProgressMessage,
  MAX_CHAT_MESSAGE_CHARS,
  resetTrainingChat,
  sendTrainingChatMessage,
  trainingChatErrorMessage,
  type InvestigationProgress,
  type RunningEvidence,
  type RunningInvestigationResponse,
  type TrainingChatResponse,
} from "./investigation";
import "./App.css";

const errorMessages: Record<PreviewErrorCode, string> = {
  preview_busy:
    "Another file operation is in progress. Try again when it finishes.",
  randomness_unavailable:
    "Effortline could not prepare this preview. Try again.",
  picker_failed: "Effortline could not open the file picker. Try again.",
  file_unavailable:
    "This selection is not a local file. Choose a FIT file on this device.",
  file_read_failed:
    "Effortline could not read that file. Check access and try again.",
  fit_too_large: "This FIT file is larger than the 16 MiB preview limit.",
  fit_too_many_records: "This FIT file has too many records to preview.",
  fit_too_many_definitions:
    "This FIT file has too many definitions to preview.",
  fit_truncated: "This FIT file is incomplete. Try another copy of the file.",
  fit_corrupt: "This FIT file is damaged or failed its checksum check.",
  fit_unsupported:
    "This FIT file uses a format or activity layout we do not support yet.",
  fit_not_activity: "This FIT file does not contain an activity with samples.",
};

function formatDuration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainingSeconds = seconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(remainingSeconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(remainingSeconds).padStart(2, "0")}`;
}

function formatDistance(distanceM: number | null): string {
  if (distanceM === null) return "Not recorded";
  if (distanceM < 1000) return `${Math.round(distanceM).toLocaleString()} m`;
  return `${(distanceM / 1000).toFixed(2)} km`;
}

function batchOutcomeMessage(outcome: BatchFileOutcome): string {
  switch (outcome.status) {
    case "saved":
      return "Saved to your encrypted library.";
    case "already_present":
      return "Already in your library. No duplicate was added.";
    case "failed":
      return outcome.code
        ? batchFileErrorMessage(outcome.code)
        : "Effortline could not save this file. Try again.";
    case "not_imported":
      return outcome.code
        ? `Not imported because the library stopped: ${batchFileErrorMessage(outcome.code)}`
        : "Not imported because you cancelled the batch.";
  }
}

function activitySportLabel(sport: BatchActivitySummary["sport"]): string {
  switch (sport) {
    case "running":
      return "Running";
    case "other":
      return "Other activity";
    case "unknown":
      return "Activity";
  }
}

type ChatTranscriptItem = {
  id: number;
  role: "user" | "assistant";
  text: string;
  kind?: "general" | "evidence" | "clarification" | "error";
  origin?: "model" | "rust";
  citations?: string[];
  evidence?: RunningInvestigationResponse | null;
};

const MAX_VISIBLE_CHAT_ITEMS = 24;

function appendChatItems(
  current: ChatTranscriptItem[],
  ...items: ChatTranscriptItem[]
): ChatTranscriptItem[] {
  return [...current, ...items].slice(-MAX_VISIBLE_CHAT_ITEMS);
}

function formatPace(secondsPerKm: number): string {
  const seconds = Math.round(secondsPerKm);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}/km`;
}

function formatDate(unixMs: number): string {
  return new Date(unixMs).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function answerText(result: RunningInvestigationResponse): string {
  if (result.status === "insufficient_data") {
    if (result.reason === "ambiguous_period_boundary") {
      return "I found enough eligible runs, but activities on both sides of the comparison boundary have the same start time. I can’t tell which period they belong in, so I won’t report a pace change.";
    }
    return result.eligible_runs === 0
      ? "I found no saved running activities with both distance and duration. Save at least six eligible runs to compare three recent runs with the three before them."
      : `I found ${result.eligible_runs} eligible ${result.eligible_runs === 1 ? "run" : "runs"}. I need ${result.required_runs} to compare three recent runs with the three before them.`;
  }
  if (result.status !== "compared") return "";
  const change = Math.abs(result.pace.change_percent).toFixed(1);
  const direction =
    result.pace.change_percent < 0
      ? `${change}% faster`
      : result.pace.change_percent > 0
        ? `${change}% slower`
        : "unchanged";
  return `Median pace was ${formatPace(result.pace.previous_median_seconds_per_km)} in the previous runs and ${formatPace(result.pace.recent_median_seconds_per_km)} in the recent runs (${direction}).`;
}

function RunningEvidenceTable({ runs }: { runs: RunningEvidence[] }) {
  return (
    <div className="investigation-evidence-wrap">
      <table className="investigation-evidence">
        <caption className="visually-hidden">Running activity evidence</caption>
        <thead>
          <tr>
            <th scope="col">Activity date</th>
            <th scope="col">Pace</th>
            <th scope="col">Distance</th>
            <th scope="col">Heart rate</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((run) => (
            <tr key={run.source_id}>
              <th scope="row">
                <span>{formatDate(run.started_at_unix_ms)}</span>
                <details>
                  <summary>Source citation</summary>
                  <code>{run.source_id}</code>
                </details>
              </th>
              <td>{formatPace(run.pace_seconds_per_km)}</td>
              <td>{formatDistance(run.distance_m)}</td>
              <td>
                {run.median_heart_rate_bpm === null
                  ? `Not recorded (${run.heart_rate_sample_count}/${run.sample_count})`
                  : `${Math.round(run.median_heart_rate_bpm)} bpm (${run.heart_rate_sample_count}/${run.sample_count})`}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function RunningAnswer({
  result,
}: {
  result: RunningInvestigationResponse;
}) {
  if (result.status === "error") {
    return <p role="alert">{saveErrorMessage(result.code)}</p>;
  }
  if (result.status === "insufficient_data") {
    return <p>{answerText(result)}</p>;
  }
  return (
    <div className="running-answer">
      <p>{answerText(result)}</p>
      <p className="evidence-intro">
        Pace uses each run’s recorded distance and elapsed time. The two groups
        contain three runs each.
      </p>
      <h3>Previous three runs</h3>
      <RunningEvidenceTable runs={result.previous_runs} />
      <h3>Recent three runs</h3>
      <RunningEvidenceTable runs={result.recent_runs} />
      {result.heart_rate.status === "available" ? (
        <p className="heart-rate-note">
          Median recorded heart rate:{" "}
          {Math.round(result.heart_rate.previous_median_bpm)} bpm in the
          previous runs and {Math.round(result.heart_rate.recent_median_bpm)}{" "}
          bpm in the recent runs.
        </p>
      ) : (
        <p className="heart-rate-note">
          Heart-rate comparison is unavailable. Only{" "}
          {result.heart_rate.previous_qualified_runs} of three previous runs and{" "}
          {result.heart_rate.recent_qualified_runs} of three recent runs have at
          least {result.heart_rate.minimum_samples_per_run} heart-rate samples,
          covering at least {result.heart_rate.minimum_coverage_percent}% of the
          run.
        </p>
      )}
      {result.device_history !== "consistent" && (
        <p className="heart-rate-note">
          {result.device_history === "mixed"
            ? "The FIT files report different devices. Device changes may affect this comparison."
            : result.device_history === "missing"
              ? "Device details are missing from these FIT files, so device changes cannot be checked."
              : "Some device details are missing and the FIT files report different devices. Device changes may affect this comparison."}
        </p>
      )}
      <p className="heart-rate-note">
        FIT files do not identify the heart-rate sensor, so sensor changes
        cannot be checked.
      </p>
      {result.explanation_error && (
        <p role="status" className="heart-rate-note">
          The measured result is ready, but the local explanation could not be
          checked. {modelErrorMessage(result.explanation_error)}
        </p>
      )}
      {result.explanation && (
        <aside className="model-interpretation">
          <h3>Possible interpretation</h3>
          <p>{result.explanation.text}</p>
          <small>
            Local model output. This is a hypothesis, not a measured result.
            Cites{" "}
            {result.explanation.citations
              .map((sourceId) =>
                [...result.previous_runs, ...result.recent_runs].find(
                  (run) => run.source_id === sourceId,
                ),
              )
              .filter((run) => run !== undefined)
              .map((run) => formatDate(run.started_at_unix_ms))
              .join(", ")}{" "}
            above.
          </small>
        </aside>
      )}
    </div>
  );
}

function batchProgressMessage(
  progress: BatchProgress | null,
  elapsedSeconds: number,
): string {
  if (!progress) return "Waiting for file selection…";
  if (progress.status === "started") {
    return `${progress.total} FIT ${progress.total === 1 ? "file" : "files"} selected.`;
  }
  if (progress.status === "file_started") {
    return `Starting ${progress.index} of ${progress.total}: ${progress.name}`;
  }
  if (progress.status === "save_stage") {
    return `${progress.index} of ${progress.total}: ${progress.name} — ${saveProgressMessage(progress.progress, elapsedSeconds)}`;
  }
  return `${progress.outcome.index} of ${progress.total} finished: ${progress.outcome.name}`;
}

export function BatchProgressView({
  processedFiles,
  totalFiles,
  active,
  saveProgress,
}: {
  processedFiles: number;
  totalFiles: number;
  active: boolean;
  saveProgress: SaveProgress | null;
}) {
  if (totalFiles < 1) return null;
  const sampleProgress =
    saveProgress?.status === "samples_written" ? saveProgress : null;
  return (
    <div className="batch-progress-card" aria-live="polite">
      <div className="progress-heading">
        <strong>Batch progress</strong>
        <span>
          {processedFiles} of {totalFiles} files processed
        </span>
      </div>
      <progress
        className="batch-file-progress"
        value={processedFiles}
        max={totalFiles}
        aria-label="Files processed"
      />
      {sampleProgress ? (
        <div className="sample-progress">
          <div className="progress-heading">
            <span>Samples added to this file</span>
            <span>
              {sampleProgress.completed.toLocaleString()} of{" "}
              {sampleProgress.total.toLocaleString()}
            </span>
          </div>
          <progress
            value={sampleProgress.completed}
            max={sampleProgress.total}
            aria-label="Samples added to the current file"
          />
          <small>Not yet committed to your library.</small>
        </div>
      ) : active ? (
        <div
          className="batch-stage-progress"
          role="progressbar"
          aria-label="Current file is being processed"
        >
          <span />
        </div>
      ) : null}
    </div>
  );
}

export function BatchFileList({ files }: { files: BatchFileOutcome[] }) {
  return (
    <div className="batch-file-table-wrap">
      <table className="batch-file-table">
        <caption className="visually-hidden">Imported FIT file details</caption>
        <thead>
          <tr>
            <th scope="col">File</th>
            <th scope="col">Duration</th>
            <th scope="col">Distance</th>
            <th scope="col">Samples</th>
            <th scope="col">Result</th>
          </tr>
        </thead>
        <tbody>
          {files
            .slice()
            .sort((a, b) => a.index - b.index)
            .map((file) => (
              <tr key={`${file.index}-${file.name}`}>
                <th scope="row" className="batch-file-name">
                  <span className="batch-file-order">{file.index}</span>
                  <span className="batch-file-info">
                    <strong>{file.name}</strong>
                    <small>
                      {file.activity
                        ? activitySportLabel(file.activity.sport)
                        : "Details unavailable"}
                    </small>
                  </span>
                </th>
                <td>
                  {file.activity
                    ? formatDuration(file.activity.duration_seconds)
                    : "—"}
                </td>
                <td>
                  {file.activity
                    ? formatDistance(file.activity.distance_m)
                    : "—"}
                </td>
                <td>
                  {file.activity
                    ? file.activity.sample_count.toLocaleString()
                    : "—"}
                </td>
                <td>
                  <span
                    className={`batch-outcome batch-outcome-${file.status}`}
                  >
                    {batchOutcomeMessage(file)}
                  </span>
                </td>
              </tr>
            ))}
        </tbody>
      </table>
    </div>
  );
}

export function BatchRetryAction({
  files,
  disabled,
  onRetry,
}: {
  files: BatchFileOutcome[];
  disabled: boolean;
  onRetry: () => void;
}) {
  const retryable = files.some(
    (file) => file.status === "failed" || file.status === "not_imported",
  );
  if (!retryable) return null;
  return (
    <div className="batch-retry-action">
      <p>
        Select the failed and not imported files again. Files already saved will
        be reported as duplicates and will not be added again.
      </p>
      <button type="button" onClick={onRetry} disabled={disabled}>
        Choose files to retry
      </button>
    </div>
  );
}

export function SaveAction({
  saving,
  busy,
  saveResponse,
  progress,
  elapsedSeconds,
  onSave,
  libraryCheck,
  onCheckAgain,
}: {
  saving: boolean;
  busy: boolean;
  saveResponse: SaveResponse | null;
  progress: SaveProgress | null;
  elapsedSeconds: number;
  onSave: () => void;
  libraryCheck?: LibraryCheckState;
  onCheckAgain?: () => void;
}) {
  if (!saveResponse && libraryCheck?.status === "checking") {
    return (
      <p role="status">
        {progress
          ? saveProgressMessage(progress, elapsedSeconds)
          : "Checking your library…"}
      </p>
    );
  }
  if (
    !saveResponse &&
    (libraryCheck?.status === "error" || libraryCheck?.status === "unavailable")
  ) {
    return (
      <div>
        <p role="alert">
          Could not check whether this activity is already saved.
          {libraryCheck.status === "error" &&
          libraryCheck.code === "library_secret_unavailable"
            ? " Allow macOS Keychain access and try again."
            : " Check library access and try again."}
        </p>
        <button type="button" onClick={onCheckAgain}>
          Check library again
        </button>
      </div>
    );
  }
  if (!saveResponse && libraryCheck?.status === "already_present") {
    saveResponse = { version: 1, status: "already_present" };
  }
  const saveButton = saveButtonState(saving, saveResponse);
  return (
    <>
      {saveResponse?.status === "saved" ||
      saveResponse?.status === "already_present" ? (
        <div className="save-complete" role="status">
          <span className="save-check" aria-hidden="true">
            ✓
          </span>
          <div>
            <h3>
              {saveResponse.status === "saved"
                ? "Saved to library"
                : "Already in your library"}
            </h3>
            <p>
              {saveResponse.status === "saved"
                ? "Your activity is encrypted and stored on this device. You can close this window."
                : "This activity was saved before. No duplicate was added."}
            </p>
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={onSave}
          disabled={busy || saveButton.disabled}
          style={{ cursor: saveButton.cursor }}
        >
          {saveButton.label}
        </button>
      )}
      {saving && (
        <p role="status">{saveProgressMessage(progress, elapsedSeconds)}</p>
      )}
    </>
  );
}

function App() {
  const [question, setQuestion] = useState("");
  const [chatMessages, setChatMessages] = useState<ChatTranscriptItem[]>([]);
  const [investigationBusy, setInvestigationBusy] = useState(false);
  const [investigationProgress, setInvestigationProgress] =
    useState<InvestigationProgress | null>(null);
  const [investigationStartedAt, setInvestigationStartedAt] = useState<
    number | null
  >(null);
  const [response, setResponse] = useState<PreviewResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [commandFailed, setCommandFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveResponse, setSaveResponse] = useState<SaveResponse | null>(null);
  const [saveCommandFailed, setSaveCommandFailed] = useState(false);
  const [batchBusy, setBatchBusy] = useState(false);
  const [batchCommandFailed, setBatchCommandFailed] = useState(false);
  const [batchResponse, setBatchResponse] = useState<BatchResponse | null>(
    null,
  );
  const [batchProgress, setBatchProgress] = useState<BatchProgress | null>(
    null,
  );
  const [batchProgressAt, setBatchProgressAt] = useState<number | null>(null);
  const [batchFiles, setBatchFiles] = useState<BatchFileOutcome[]>([]);
  const [batchId, setBatchId] = useState<string | null>(null);
  const [batchCancelRequested, setBatchCancelRequested] = useState(false);
  const [modelStatus, setModelStatus] = useState<LocalModelStatus | null>(null);
  const [modelProgress, setModelProgress] = useState<LocalModelProgress | null>(
    null,
  );
  const [modelBusy, setModelBusy] = useState(false);
  const [modelInstalling, setModelInstalling] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  const [saveProgress, setSaveProgress] = useState<{
    event: SaveProgress;
    startedAt: number;
  } | null>(null);
  const [libraryCheck, setLibraryCheck] = useState<LibraryCheckState>({
    status: "checking",
  });
  const checkingLibrary =
    libraryCheck.status === "checking" && response?.status === "ready";
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    let active = true;
    void getLocalModelStatus()
      .then((status) => {
        if (active) setModelStatus(status);
      })
      .catch((error: unknown) => {
        if (active) setModelError(modelErrorMessage(error));
      });
    return () => {
      active = false;
    };
  }, []);

  async function installModel() {
    if (modelBusy) return;
    setModelBusy(true);
    setModelInstalling(true);
    setModelError(null);
    try {
      const status = await installLocalModel(setModelProgress);
      setModelStatus(status);
      setModelProgress({
        downloaded_bytes: status.download_bytes,
        total_bytes: status.download_bytes,
        stage: "installed",
      });
    } catch (error) {
      setModelError(modelErrorMessage(error));
      setModelProgress(null);
    } finally {
      setModelBusy(false);
      setModelInstalling(false);
    }
  }

  async function cancelModelInstall() {
    try {
      await cancelLocalModelInstall();
    } catch (error) {
      setModelError(modelErrorMessage(error));
    }
  }

  async function removeModel() {
    if (
      !window.confirm(
        "Remove the local model? Your activity library will not change.",
      )
    )
      return;
    setModelBusy(true);
    setModelError(null);
    try {
      await removeLocalModel();
      setModelStatus((status) =>
        status ? { ...status, installed: false } : status,
      );
      setModelProgress(null);
    } catch (error) {
      setModelError(modelErrorMessage(error));
    } finally {
      setModelBusy(false);
    }
  }

  async function submitQuestion(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const message = question.trim();
    if (investigationBusy || message.length === 0) return;
    const id = Date.now();
    setChatMessages((current) =>
      appendChatItems(current, { id, role: "user", text: message }),
    );
    setQuestion("");
    setInvestigationProgress(null);
    const startedAt = Date.now();
    setNow(startedAt);
    setInvestigationStartedAt(startedAt);
    setInvestigationBusy(true);
    try {
      const result: TrainingChatResponse = await sendTrainingChatMessage(
        message,
        (progress) => {
          setInvestigationProgress(progress);
          setNow(Date.now());
        },
      );
      if (result.status === "turn") {
        setChatMessages((current) =>
          appendChatItems(current, {
            id: id + 1,
            role: "assistant",
            text: result.reply.text,
            kind: result.reply.kind,
            origin: result.reply.origin,
            citations: result.reply.citations,
            evidence: result.evidence,
          }),
        );
      } else {
        setChatMessages((current) =>
          appendChatItems(current, {
            id: id + 1,
            role: "assistant",
            text: trainingChatErrorMessage(result.code),
            kind: "error",
          }),
        );
      }
    } catch {
      setChatMessages((current) =>
        appendChatItems(current, {
          id: id + 1,
          role: "assistant",
          text: "Effortline could not complete this message. Try again.",
          kind: "error",
        }),
      );
    } finally {
      setInvestigationBusy(false);
      setInvestigationStartedAt(null);
    }
  }

  async function startNewChat() {
    if (investigationBusy) return;
    try {
      await resetTrainingChat();
      setChatMessages([]);
      setInvestigationProgress(null);
    } catch {
      setChatMessages((current) =>
        appendChatItems(current, {
          id: Date.now(),
          role: "assistant",
          text: "Effortline could not start a new chat. Try again.",
          kind: "error",
        }),
      );
    }
  }

  useEffect(() => {
    if (!saving && !checkingLibrary && !batchBusy && !investigationBusy) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [saving, checkingLibrary, batchBusy, investigationBusy]);

  async function saveActivity() {
    if (
      response?.status !== "ready" ||
      busy ||
      batchBusy ||
      saving ||
      libraryCheck.status !== "not_present"
    )
      return;
    setSaveProgress(null);
    setSaving(true);
    setSaveResponse(null);
    setSaveCommandFailed(false);
    try {
      setSaveResponse(
        await saveFitPreview(response.preview_id, (event) => {
          const receivedAt = Date.now();
          setNow(receivedAt);
          setSaveProgress({ event, startedAt: receivedAt });
        }),
      );
    } catch {
      setSaveCommandFailed(true);
    } finally {
      setSaving(false);
    }
  }

  async function checkLibrary(previewId: string) {
    setLibraryCheck({ status: "checking" });
    setSaveProgress(null);
    try {
      setLibraryCheck(
        await checkFitPreview(previewId, (event) => {
          const receivedAt = Date.now();
          setNow(receivedAt);
          setSaveProgress({ event, startedAt: receivedAt });
        }),
      );
    } catch {
      setLibraryCheck({ status: "unavailable" });
    }
  }

  async function chooseFitFile() {
    if (busy || saving || checkingLibrary || batchBusy || investigationBusy)
      return;
    setBusy(true);
    setSaveResponse(null);
    setSaveCommandFailed(false);
    setResponse(null);
    setCommandFailed(false);
    try {
      const next = await requestFitPreview();
      setResponse(next);
      if (next.status === "ready") await checkLibrary(next.preview_id);
    } catch {
      setCommandFailed(true);
    } finally {
      setBusy(false);
    }
  }

  async function importMultipleFitFiles() {
    if (busy || saving || checkingLibrary || batchBusy || investigationBusy)
      return;
    const hasPreviousResults = batchFiles.length > 0;
    setBatchBusy(true);
    setBatchCommandFailed(false);
    setBatchProgress(null);
    setBatchProgressAt(null);
    setBatchId(null);
    setBatchCancelRequested(false);
    try {
      const result = await importFitFiles((progress) => {
        setBatchProgress(progress);
        setBatchProgressAt(Date.now());
        if (progress.status === "started") {
          setBatchId(progress.batch_id);
          if (hasPreviousResults) {
            setBatchResponse(null);
            setBatchFiles([]);
          }
        }
        if (progress.status === "file_finished") {
          setBatchFiles((current) => [
            ...current.filter((file) => file.index !== progress.outcome.index),
            progress.outcome,
          ]);
        }
      });
      if (result.status === "completed") {
        setBatchResponse(result);
        setBatchFiles(result.files);
        if (response?.status === "ready") {
          void checkLibrary(response.preview_id);
        }
      } else if (result.status === "picker_cancelled" && !hasPreviousResults) {
        setBatchResponse(result);
      } else if (result.status === "error") {
        setBatchResponse(result);
      }
    } catch {
      setBatchCommandFailed(true);
    } finally {
      setBatchBusy(false);
      setBatchId(null);
    }
  }

  async function cancelBatch() {
    if (!batchId || batchCancelRequested) return;
    setBatchCancelRequested(true);
    try {
      const response = await cancelFitImport(batchId);
      if (response.status !== "accepted") setBatchCancelRequested(false);
    } catch {
      setBatchCancelRequested(false);
    }
  }

  return (
    <main className="app-shell">
      <p className="eyebrow">EFFORTLINE · TRAINING INVESTIGATION</p>
      <h1>Ask about your training.</h1>
      <p className="intro">
        Get answers from saved activities, with the evidence behind each result.
      </p>
      <section className="local-model-card" aria-label="Local model">
        <div>
          <strong>Local training assistant</strong>
          <p>
            Install a model to chat about your training. It runs on this Mac.
          </p>
          <small>
            Download about 1.28 GB. Keep about 1.3 GB of free storage.
          </small>
        </div>
        {modelInstalling ? (
          <button type="button" onClick={() => void cancelModelInstall()}>
            Cancel download
          </button>
        ) : modelBusy ? (
          <button type="button" disabled>
            Removing model…
          </button>
        ) : modelStatus?.installed ? (
          <button type="button" onClick={() => void removeModel()}>
            Remove model
          </button>
        ) : (
          <button type="button" onClick={() => void installModel()}>
            Install model
          </button>
        )}
        {modelInstalling && modelProgress && (
          <div className="model-download-progress" aria-live="polite">
            <progress
              value={modelProgress.downloaded_bytes}
              max={modelProgress.total_bytes}
              aria-label="Model download progress"
            />
            <small>
              {modelProgress.stage === "verifying"
                ? "Checking the model download…"
                : `${(modelProgress.downloaded_bytes / 1_000_000).toFixed(0)} of ${(modelProgress.total_bytes / 1_000_000).toFixed(0)} MB downloaded`}
            </small>
          </div>
        )}
        {modelStatus?.installed && !modelBusy && (
          <small>
            The model is installed. Your training chat runs on this Mac.
          </small>
        )}
        {modelError && <p role="alert">{modelError}</p>}
      </section>
      <form
        className="question-composer"
        onSubmit={(event) => void submitQuestion(event)}
      >
        <label htmlFor="running-question">Your message</label>
        <input
          id="running-question"
          value={question}
          onChange={(event) => setQuestion(event.currentTarget.value)}
          placeholder="Ask about your activities or training in general"
          maxLength={MAX_CHAT_MESSAGE_CHARS}
          disabled={investigationBusy}
        />
        <button
          type="submit"
          disabled={investigationBusy || question.trim().length === 0}
        >
          {investigationBusy ? "Working…" : "Ask"}
        </button>
      </form>
      {chatMessages.length === 0 && (
        <p className="chat-empty-note">
          Ask a question about your training. Effortline can check saved running
          activities or discuss general training ideas.
        </p>
      )}
      {chatMessages.length > 0 && (
        <section
          className="result conversation"
          aria-live="polite"
          aria-busy={investigationBusy}
        >
          <div className="chat-heading">
            <h2>Training chat</h2>
            <button
              type="button"
              onClick={() => void startNewChat()}
              disabled={investigationBusy}
            >
              New chat
            </button>
          </div>
          {chatMessages.map((item) =>
            item.role === "user" ? (
              <p className="user-question" key={item.id}>
                {item.text}
              </p>
            ) : (
              <div className="assistant-answer" key={item.id}>
                {item.kind === "evidence" && (
                  <small className="chat-reply-origin">
                    {item.origin === "model"
                      ? "Possible interpretation · local model"
                      : "Measured summary · Effortline"}
                  </small>
                )}
                <p>{item.text}</p>
                {item.kind === "evidence" && item.evidence && (
                  <RunningAnswer result={item.evidence} />
                )}
                {item.kind === "evidence" &&
                  item.citations &&
                  item.citations.length > 0 && (
                    <small className="chat-citations">
                      Evidence checked: {item.citations.length} activities
                    </small>
                  )}
              </div>
            ),
          )}
          {investigationBusy && (
            <p role="status">
              {investigationProgressMessage(
                investigationProgress,
                investigationStartedAt === null
                  ? 0
                  : (now - investigationStartedAt) / 1000,
              )}
            </p>
          )}
        </section>
      )}

      <h2 className="library-heading">Add activities to your library</h2>
      <p className="intro">
        FIT files stay on this device. Effortline keeps the original bytes
        encrypted in your local library.
      </p>
      <div className="import-actions" aria-label="Choose an import action">
        <button
          className="primary-action"
          type="button"
          onClick={() => void chooseFitFile()}
          disabled={
            busy || saving || checkingLibrary || batchBusy || investigationBusy
          }
        >
          {checkingLibrary
            ? "Checking library…"
            : busy
              ? "Opening file…"
              : "Preview one FIT file"}
        </button>

        <button
          className="secondary-action"
          type="button"
          onClick={() => void importMultipleFitFiles()}
          disabled={
            busy || saving || checkingLibrary || batchBusy || investigationBusy
          }
        >
          {batchBusy ? "Importing FIT files…" : "Import several FIT files"}
        </button>
      </div>

      <section
        className="result"
        aria-live="polite"
        aria-busy={busy && !checkingLibrary}
      >
        {busy && !checkingLibrary && (
          <p>Choose a file in the system window to see its preview.</p>
        )}
        {response?.status === "cancelled" && (
          <p>No file was selected. No activity was saved.</p>
        )}
        {response?.status === "error" && (
          <div role="alert">
            <h2>Could not preview this activity</h2>
            <p>
              {errorMessages[response.code] ??
                "Effortline could not read this FIT file."}
            </p>
          </div>
        )}
        {commandFailed && (
          <div role="alert">
            <h2>Preview is unavailable</h2>
            <p>Effortline could not start the preview. Try again.</p>
          </div>
        )}
        {response?.status === "ready" && (
          <div>
            <h2>Activity preview</h2>
            <dl className="summary">
              <div>
                <dt>Sport</dt>
                <dd>
                  {response.preview.sport === "running"
                    ? "Running"
                    : response.preview.sport === "other"
                      ? "Other"
                      : "Unknown"}
                </dd>
              </div>
              <div>
                <dt>Elapsed time</dt>
                <dd>{formatDuration(response.preview.duration_seconds)}</dd>
              </div>
              <div>
                <dt>Distance</dt>
                <dd>{formatDistance(response.preview.distance_m)}</dd>
              </div>
              <div>
                <dt>Samples</dt>
                <dd>{response.preview.sample_count.toLocaleString()}</dd>
              </div>
              <div>
                <dt>Heart rate</dt>
                <dd>
                  {response.preview.heart_rate_samples > 0
                    ? `Available in ${response.preview.heart_rate_samples.toLocaleString()} of ${response.preview.sample_count.toLocaleString()} samples`
                    : "Not recorded"}
                </dd>
              </div>
            </dl>
            <SaveAction
              saving={saving}
              busy={busy || batchBusy}
              saveResponse={saveResponse}
              progress={saveProgress?.event ?? null}
              elapsedSeconds={
                saveProgress ? (now - saveProgress.startedAt) / 1000 : 0
              }
              onSave={() => void saveActivity()}
              libraryCheck={libraryCheck}
              onCheckAgain={() => void checkLibrary(response.preview_id)}
            />
            {saveResponse?.status === "error" && (
              <p role="alert">{saveErrorMessage(saveResponse.code)}</p>
            )}
            {saveCommandFailed && (
              <p role="alert">
                Effortline could not confirm the save. Try again. Retrying will
                not create a duplicate.
              </p>
            )}
            {!saving &&
              !saveResponse &&
              !saveCommandFailed &&
              libraryCheck.status === "not_present" && (
                <p className="save-note">
                  This preview has not been saved. Select “Save to library” to
                  keep an encrypted copy.
                </p>
              )}
          </div>
        )}
      </section>

      {(batchBusy || batchResponse || batchCommandFailed) && (
        <section
          className="result batch-result"
          aria-live="polite"
          aria-busy={batchBusy}
        >
          <h2>Import multiple FIT files</h2>
          {batchBusy && (
            <>
              <p role="status">
                {batchProgress
                  ? batchProgressMessage(
                      batchProgress,
                      batchProgressAt ? (now - batchProgressAt) / 1000 : 0,
                    )
                  : "Choose FIT files in the system window. Cancel there to make no changes."}
              </p>
              {batchId && (
                <button
                  type="button"
                  onClick={() => void cancelBatch()}
                  disabled={batchCancelRequested}
                >
                  {batchCancelRequested
                    ? "Stopping after this file…"
                    : "Cancel after this file"}
                </button>
              )}
            </>
          )}
          {(batchProgress?.total || batchResponse?.status === "completed") && (
            <BatchProgressView
              processedFiles={batchFiles.length}
              totalFiles={
                batchProgress?.total ??
                (batchResponse?.status === "completed"
                  ? batchResponse.files.length
                  : 0)
              }
              active={batchBusy}
              saveProgress={
                batchProgress?.status === "save_stage"
                  ? batchProgress.progress
                  : null
              }
            />
          )}
          {batchCommandFailed && (
            <p role="alert">
              Effortline could not start the batch import. Try again.
            </p>
          )}
          {batchResponse?.status === "picker_cancelled" && (
            <p>No files selected. Nothing was imported.</p>
          )}
          {batchResponse?.status === "error" && (
            <p role="alert">{batchCommandErrorMessage(batchResponse.code)}</p>
          )}
          {batchFiles.length > 0 && (
            <>
              <BatchFileList files={batchFiles} />
              {!batchBusy && (
                <BatchRetryAction
                  files={batchFiles}
                  disabled={batchBusy || busy || saving || checkingLibrary}
                  onRetry={() => void importMultipleFitFiles()}
                />
              )}
              {batchResponse?.status === "completed" &&
                batchResponse.cancelled && (
                  <p role="status">
                    Cancelled after the current file. Completed files remain
                    saved.
                  </p>
                )}
            </>
          )}
        </section>
      )}
    </main>
  );
}

export default App;
