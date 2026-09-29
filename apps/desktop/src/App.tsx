import { useState } from "react";
import {
  requestFitPreview,
  type PreviewErrorCode,
  type PreviewResponse,
} from "./fitPreview";
import "./App.css";

const errorMessages: Record<PreviewErrorCode, string> = {
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

function App() {
  const [response, setResponse] = useState<PreviewResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [commandFailed, setCommandFailed] = useState(false);

  async function chooseFitFile() {
    setBusy(true);
    setResponse(null);
    setCommandFailed(false);
    try {
      setResponse(await requestFitPreview());
    } catch {
      setCommandFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <p className="eyebrow">EFFORTLINE · ACTIVITY PREVIEW</p>
      <h1>See what your FIT file contains.</h1>
      <p className="intro">
        Choose one activity file. Effortline will read it on this device and
        show a short summary.
      </p>
      <button
        type="button"
        onClick={() => void chooseFitFile()}
        disabled={busy}
      >
        {busy ? "Opening file…" : "Choose a FIT file"}
      </button>

      <section className="result" aria-live="polite" aria-busy={busy}>
        {busy && <p>Choose a file in the system window to see its preview.</p>}
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
          </div>
        )}
      </section>
      <p className="save-note">
        Preview only. This activity has not been saved.
      </p>
    </main>
  );
}

export default App;
