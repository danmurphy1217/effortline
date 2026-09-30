import { invoke } from "@tauri-apps/api/core";

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

export async function saveFitPreview(previewId: string): Promise<SaveResponse> {
  const response = await invoke<SaveResponse>("save_preview_to_library", {
    request: { version: 1, preview_id: previewId },
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
