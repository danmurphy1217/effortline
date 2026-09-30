import { useEffect, useState } from "react";
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
  type BatchProgress,
  type BatchResponse,
  type BatchFileOutcome,
} from "./librarySave";
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

export function BatchFileList({ files }: { files: BatchFileOutcome[] }) {
  return (
    <ul className="batch-file-list">
      {files
        .slice()
        .sort((a, b) => a.index - b.index)
        .map((file) => (
          <li key={`${file.index}-${file.name}`}>
            <strong>
              {file.index}. {file.name}
            </strong>
            <span>{batchOutcomeMessage(file)}</span>
          </li>
        ))}
    </ul>
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
    if (!saving && !checkingLibrary && !batchBusy) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [saving, checkingLibrary, batchBusy]);

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
    if (busy || saving || checkingLibrary || batchBusy) return;
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
    if (busy || saving || checkingLibrary || batchBusy) return;
    setBatchBusy(true);
    setBatchCommandFailed(false);
    setBatchResponse(null);
    setBatchProgress(null);
    setBatchProgressAt(null);
    setBatchFiles([]);
    setBatchId(null);
    setBatchCancelRequested(false);
    try {
      const result = await importFitFiles((progress) => {
        setBatchProgress(progress);
        setBatchProgressAt(Date.now());
        if (progress.status === "started") setBatchId(progress.batch_id);
        if (progress.status === "file_finished") {
          setBatchFiles((current) => [
            ...current.filter((file) => file.index !== progress.outcome.index),
            progress.outcome,
          ]);
        }
      });
      setBatchResponse(result);
      if (result.status === "completed") setBatchFiles(result.files);
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
      <p className="eyebrow">EFFORTLINE · ACTIVITY PREVIEW</p>
      <h1>See what your FIT file contains.</h1>
      <p className="intro">
        Choose one activity file. Effortline will read it on this device and
        show a short summary, or import several FIT files into your encrypted
        local library.
      </p>
      <button
        type="button"
        onClick={() => void chooseFitFile()}
        disabled={busy || saving || checkingLibrary || batchBusy}
      >
        {checkingLibrary
          ? "Checking library…"
          : busy
            ? "Opening file…"
            : "Choose a FIT file"}
      </button>

      <button
        type="button"
        onClick={() => void importMultipleFitFiles()}
        disabled={busy || saving || checkingLibrary || batchBusy}
      >
        {batchBusy ? "Importing FIT files…" : "Import multiple FIT files"}
      </button>

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
          {batchCommandFailed && (
            <p role="alert">
              Effortline could not start the batch import. Try again.
            </p>
          )}
          {batchResponse?.status === "picker_cancelled" && (
            <p>No files selected. Nothing was imported.</p>
          )}
          {batchResponse?.status === "error" && (
            <p role="alert">
              {batchResponse.code === "too_many_files"
                ? "Choose no more than 32 FIT files at a time."
                : batchResponse.code === "library_busy"
                  ? "The library is busy. Wait for the other operation to finish, then retry."
                  : batchResponse.code === "library_location_unavailable"
                    ? "Effortline could not locate the library. Check disk access and retry."
                    : "Effortline could not start the file picker. Try again."}
            </p>
          )}
          {batchFiles.length > 0 && (
            <>
              <BatchFileList files={batchFiles} />
              {!batchBusy &&
                batchFiles.some((file) => file.status === "failed") && (
                  <p>
                    You can select the failed files again. Files already saved
                    will be reported as already in your library.
                  </p>
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
