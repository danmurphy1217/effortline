import { invoke } from "@tauri-apps/api/core";
import type { SaveErrorCode } from "./librarySave";

export type RunningEvidence = {
  source_id: string;
  started_at_unix_ms: number;
  duration_seconds: number;
  distance_m: number;
  pace_seconds_per_km: number;
  sample_count: number;
  heart_rate_sample_count: number;
  median_heart_rate_bpm: number | null;
};

export type HeartRateResult =
  | {
      status: "available";
      previous_median_bpm: number;
      recent_median_bpm: number;
    }
  | {
      status: "insufficient_coverage";
      previous_qualified_runs: number;
      recent_qualified_runs: number;
      required_runs_per_period: number;
      minimum_samples_per_run: number;
      minimum_coverage_percent: number;
    };

export type RunningInvestigationResponse =
  | {
      version: 1;
      status: "compared";
      previous_runs: RunningEvidence[];
      recent_runs: RunningEvidence[];
      pace: {
        previous_median_seconds_per_km: number;
        recent_median_seconds_per_km: number;
        change_percent: number;
      };
      heart_rate: HeartRateResult;
    }
  | {
      version: 1;
      status: "insufficient_data";
      eligible_runs: number;
      required_runs: number;
      reason: "too_few_runs" | "ambiguous_period_boundary";
    }
  | { version: 1; status: "error"; code: SaveErrorCode };

export async function askRunningChange(): Promise<RunningInvestigationResponse> {
  const response = await invoke<RunningInvestigationResponse>(
    "investigate_recent_running",
    { request: { version: 1 } },
  );
  if (response.version !== 1)
    throw new Error("Unsupported investigation response version");
  return response;
}
