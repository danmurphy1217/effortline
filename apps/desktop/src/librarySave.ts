import { Channel, invoke } from "@tauri-apps/api/core";

type SaveErrorCode =
  | "invalid_preview"
  | "unsupported_request"
  | "preview_expired"
  | "library_location_unavailable"
  | "library_secret_unavailable"
  | "library_busy"
  | "library_cannot_unlock"
  | "library_encryption_unavailable"
  | "library_unsupported_schema"
  | "library_incomplete"
  | "library_corrupt_original"
  | "library_randomness_unavailable"
  | "library_io"
  | "library_database"
  | "save_failed";

export type SaveResponse =
  | { version: 1; status: "saved" | "already_present" }
  | { version: 1; status: "error"; code: SaveErrorCode };

export type SaveStage =
  | "keychain_access"
  | "open_recovery"
  | "fit_parsing"
  | "duplicate_check"
  | "encryption"
  | "file_write_sync"
  | "sample_inserts"
  | "database_commit";

export type SaveProgress =
  | {
      version: 1;
      status: "samples_written";
      completed: number;
      total: number;
      elapsed_ms: number;
    }
  | { version: 1; status: "started"; stage: SaveStage }
  | {
      version: 1;
      status: "finished";
      stage: SaveStage;
      elapsed_ms: number;
      succeeded: boolean;
    };

const stageMessages: Record<SaveStage, string> = {
  keychain_access: "Accessing the library key in macOS Keychain",
  open_recovery: "Opening the encrypted library and checking stored files",
  fit_parsing: "Reading the FIT activity",
  duplicate_check: "Checking for an existing activity",
  encryption: "Encrypting the original FIT file",
  file_write_sync: "Writing and syncing the encrypted file",
  sample_inserts: "Saving activity samples",
  database_commit: "Committing the activity to the library",
};

export function saveProgressMessage(
  progress: SaveProgress | null,
  elapsedSeconds: number,
): string {
  if (!progress) return "Starting the save…";
  if (progress.status === "samples_written") {
    const elapsed = Math.floor(
      progress.elapsed_ms / 1000 + Math.max(0, elapsedSeconds),
    );
    return `Saving activity samples — ${progress.completed.toLocaleString()} of ${progress.total.toLocaleString()} written (${elapsed} s). Not yet committed to your library.`;
  }
  if (progress.status === "finished") {
    return progress.succeeded
      ? `${stageMessages[progress.stage]} completed. Waiting for the next save result.`
      : `${stageMessages[progress.stage]} failed. Waiting for error details.`;
  }
  const elapsed = Math.max(0, Math.floor(elapsedSeconds));
  const action =
    progress.stage === "keychain_access"
      ? " Check for a macOS Keychain permission window. The library has not been opened yet."
      : elapsed >= 10
        ? " This stage is taking longer than expected. Completion is not yet confirmed."
        : "";
  return `${stageMessages[progress.stage]} — ${elapsed} s.${action}`;
}

export function saveButtonState(
  saving: boolean,
  response: SaveResponse | null,
) {
  const complete =
    response?.status === "saved" || response?.status === "already_present";
  return {
    disabled: saving || complete,
    cursor: saving ? "progress" : "default",
    label: saving
      ? "Saving…"
      : response?.status === "saved"
        ? "Saved"
        : response?.status === "already_present"
          ? "Already saved"
          : "Save to library",
  };
}

export async function saveFitPreview(
  previewId: string,
  onProgress: (progress: SaveProgress) => void,
): Promise<SaveResponse> {
  const channel = new Channel<SaveProgress>();
  channel.onmessage = (progress) => {
    if (progress.version === 1) onProgress(progress);
  };
  const response = await invoke<SaveResponse>("save_preview_to_library", {
    request: { version: 1, preview_id: previewId },
    onProgress: channel,
  });
  if (response.version !== 1) {
    throw new Error("Unsupported save response version");
  }
  return response;
}

export function saveErrorMessage(code: SaveErrorCode): string {
  switch (code) {
    case "preview_expired":
      return "This preview is no longer available. Choose the FIT file again.";
    case "library_secret_unavailable":
      return "Effortline could not access the library key in macOS Keychain. Allow Keychain access and try again. An existing library needs its original key.";
    case "library_busy":
      return "The library is busy. Wait for the other operation to finish, then try again.";
    case "library_cannot_unlock":
      return "Effortline could not unlock the library with its stored key.";
    case "library_unsupported_schema":
      return "This library uses an unsupported format. Open it with the version of Effortline that created it.";
    case "library_incomplete":
    case "library_corrupt_original":
      return "The library has a missing or damaged file. Effortline could not save this activity.";
    default:
      return "Effortline could not confirm the save. Check disk space and access, then try again. Retrying the same file will not create a duplicate.";
  }
}
