import { invoke } from "@tauri-apps/api/core";

export type PreviewErrorCode =
  | "picker_failed"
  | "preview_busy"
  | "randomness_unavailable"
  | "file_unavailable"
  | "file_read_failed"
  | "fit_too_large"
  | "fit_too_many_records"
  | "fit_too_many_definitions"
  | "fit_truncated"
  | "fit_corrupt"
  | "fit_unsupported"
  | "fit_not_activity";

export type FitPreview = {
  sport: "running" | "other" | "unknown";
  duration_seconds: number;
  distance_m: number | null;
  sample_count: number;
  heart_rate_samples: number;
};

export type PreviewResponse =
  | { version: 1; status: "ready"; preview_id: string; preview: FitPreview }
  | { version: 1; status: "cancelled" }
  | { version: 1; status: "error"; code: PreviewErrorCode };

export async function requestFitPreview(): Promise<PreviewResponse> {
  const response = await invoke<PreviewResponse>("preview_fit_activity");
  if (response.version !== 1) {
    throw new Error("Unsupported preview response version");
  }
  return response;
}
