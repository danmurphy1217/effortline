import { Channel, invoke } from "@tauri-apps/api/core";

export type SaveErrorCode =
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
  | "save_failed"
  | "fit_too_large"
  | "fit_too_many_records"
  | "fit_too_many_definitions"
  | "fit_truncated"
  | "fit_corrupt"
  | "fit_unsupported"
  | "fit_not_activity"
  | "file_read_failed";

export type BatchFileErrorCode = SaveErrorCode;

export type BatchFileOutcome = {
  index: number;
  name: string;
  status: "saved" | "already_present" | "failed" | "not_imported";
  code: BatchFileErrorCode | null;
};

export type BatchProgress =
  | { version: 1; status: "started"; batch_id: string; total: number }
  | {
      version: 1;
      status: "file_started";
      batch_id: string;
      index: number;
      total: number;
      name: string;
    }
  | {
      version: 1;
      status: "save_stage";
      batch_id: string;
      index: number;
      total: number;
      name: string;
      progress: SaveProgress;
    }
  | {
      version: 1;
      status: "file_finished";
      batch_id: string;
      total: number;
      outcome: BatchFileOutcome;
    };

export type BatchResponse =
  | { version: 1; status: "picker_cancelled" }
  | {
      version: 1;
      status: "error";
      code:
        | "picker_failed"
        | "too_many_files"
        | "library_busy"
        | "library_location_unavailable"
        | "randomness_unavailable";
    }
  | {
      version: 1;
      status: "completed";
      batch_id: string;
      cancelled: boolean;
      files: BatchFileOutcome[];
    };

export type CancelBatchResponse =
  | { version: 1; status: "accepted" | "not_running" }
  | { version: 1; status: "error" };

export async function importFitFiles(
  onProgress: (progress: BatchProgress) => void,
): Promise<BatchResponse> {
  const channel = new Channel<BatchProgress>();
  channel.onmessage = (progress) => {
    if (progress.version === 1) onProgress(progress);
  };
  const response = await invoke<BatchResponse>("import_fit_files", {
    onProgress: channel,
  });
  if (response.version !== 1)
    throw new Error("Unsupported batch import response version");
  return response;
}

export async function cancelFitImport(
  batchId: string,
): Promise<CancelBatchResponse> {
  return invoke<CancelBatchResponse>("cancel_fit_import", {
    request: { version: 1, batch_id: batchId },
  });
}

export type SaveResponse =
  | { version: 1; status: "saved" | "already_present" }
  | { version: 1; status: "error"; code: SaveErrorCode };

export type LibraryCheckResponse =
  | { version: 1; status: "already_present" | "not_present" }
  | { version: 1; status: "error"; code: SaveErrorCode };

export type LibraryCheckState =
  LibraryCheckResponse | { status: "checking" | "unavailable" };

export async function checkFitPreview(
  previewId: string,
  onProgress: (progress: SaveProgress) => void,
): Promise<LibraryCheckResponse> {
  const channel = new Channel<SaveProgress>();
  channel.onmessage = (progress) => {
    if (progress.version === 1) onProgress(progress);
  };
  const response = await invoke<LibraryCheckResponse>(
    "check_preview_in_library",
    {
      request: { version: 1, preview_id: previewId },
      onProgress: channel,
    },
  );
  if (response.version !== 1)
    throw new Error("Unsupported library check response version");
  return response;
}

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
      ? `${stageMessages[progress.stage]} completed. Waiting for the result.`
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

export function batchFileErrorMessage(code: BatchFileErrorCode): string {
  switch (code) {
    case "file_read_failed":
      return "Effortline could not read this file. Check file access and try again.";
    case "fit_too_large":
      return "This FIT file is larger than the 16 MiB limit.";
    case "fit_too_many_records":
      return "This FIT file has too many records to import.";
    case "fit_too_many_definitions":
      return "This FIT file has too many definitions to import.";
    case "fit_truncated":
      return "This FIT file is incomplete. Try another copy of the file.";
    case "fit_corrupt":
      return "This FIT file is damaged or failed its checksum check.";
    case "fit_unsupported":
      return "This FIT file uses a format or activity layout Effortline does not support yet.";
    case "fit_not_activity":
      return "This FIT file does not contain an activity with samples.";
    case "library_secret_unavailable":
      return "Effortline could not access the library key in macOS Keychain. Allow access and retry.";
    case "library_cannot_unlock":
    case "library_encryption_unavailable":
      return "Effortline could not unlock the encrypted library.";
    case "library_busy":
      return "The library is busy. Wait for the operation to finish, then retry.";
    case "library_unsupported_schema":
      return "This library uses an unsupported format. Open it with the version of Effortline that created it.";
    case "library_incomplete":
    case "library_corrupt_original":
      return "The library has a missing or damaged file. Effortline stopped this batch.";
    case "library_location_unavailable":
      return "Effortline could not locate the library. Check disk access and retry.";
    case "library_randomness_unavailable":
      return "Effortline could not prepare encrypted storage. Try again.";
    case "library_io":
    case "library_database":
    case "save_failed":
    case "invalid_preview":
    case "unsupported_request":
    case "preview_expired":
      return "Effortline could not save this activity. Check disk space and access, then retry.";
  }
}
