import { Channel, invoke } from "@tauri-apps/api/core";
import { saveErrorMessage, type SaveErrorCode } from "./librarySave.ts";

export const MAX_CHAT_MESSAGE_CHARS = 800;

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

export type GeneratedExplanation = { text: string; citations: string[] };

export type InvestigationStage =
  | "opening_library"
  | "analyzing_activities"
  | "verifying_model"
  | "loading_model"
  | "generating_explanation";

export type InvestigationProgress = {
  version: 1;
  stage: InvestigationStage;
  elapsed_ms: number;
};

export type InvestigationRequest = { version: 1 };

export type TrainingChatReply = {
  text: string;
  kind: "general" | "evidence" | "clarification";
  origin: "model" | "rust";
  citations: string[];
};

export type TrainingChatError =
  | {
      code:
        | "invalid_request"
        | "busy"
        | "model_not_installed"
        | "model_unavailable"
        | "unsafe_model_response";
    }
  | { code: "library"; detail: SaveErrorCode };

export type TrainingChatResponse =
  | {
      version: 1;
      status: "turn";
      reply: TrainingChatReply;
      evidence: RunningInvestigationResponse | null;
    }
  | { version: 1; status: "error"; code: TrainingChatError };

const stageMessages: Record<InvestigationStage, string> = {
  opening_library: "Opening your encrypted activity library",
  analyzing_activities: "Comparing your saved running activities",
  verifying_model: "Verifying the installed model on this Mac",
  loading_model: "Loading the local model into memory",
  generating_explanation: "Generating a local explanation from the evidence",
};

export function investigationProgressMessage(
  progress: InvestigationProgress | null,
  elapsedSeconds: number,
): string {
  if (!progress) return "Preparing a private response…";
  const elapsed = Math.max(0, Math.floor(elapsedSeconds));
  const keychainHint =
    progress.stage === "opening_library" && elapsed >= 10
      ? " Check for a macOS Keychain permission window."
      : "";
  return `${stageMessages[progress.stage]} — ${elapsed} s.${keychainHint}`;
}

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
      device_history: "consistent" | "mixed" | "missing" | "mixed_or_missing";
      explanation: GeneratedExplanation | null;
      explanation_error:
        | "verification_failed"
        | "install_in_progress"
        | "location_unavailable"
        | "model_load_failed"
        | "inference_failed"
        | null;
    }
  | {
      version: 1;
      status: "insufficient_data";
      eligible_runs: number;
      required_runs: number;
      reason: "too_few_runs" | "ambiguous_period_boundary";
    }
  | { version: 1; status: "error"; code: SaveErrorCode };

export async function askRunningChange(
  onProgress: (progress: InvestigationProgress) => void,
): Promise<RunningInvestigationResponse> {
  const request: InvestigationRequest = { version: 1 };
  const channel = new Channel<InvestigationProgress>();
  channel.onmessage = (progress) => {
    if (progress.version === 1) onProgress(progress);
  };
  const response = await invoke<RunningInvestigationResponse>(
    "investigate_recent_running",
    { request, progress: channel },
  );
  if (response.version !== 1)
    throw new Error("Unsupported investigation response version");
  return response;
}

export async function sendTrainingChatMessage(
  message: string,
  onProgress: (progress: InvestigationProgress) => void,
): Promise<TrainingChatResponse> {
  const channel = new Channel<InvestigationProgress>();
  channel.onmessage = (progress) => {
    if (progress.version === 1) onProgress(progress);
  };
  const response = await invoke<TrainingChatResponse>("training_chat", {
    request: { version: 1, message },
    progress: channel,
  });
  if (response.version !== 1)
    throw new Error("Unsupported training chat response version");
  return response;
}

export async function resetTrainingChat(): Promise<void> {
  const reset = await invoke<boolean>("reset_training_chat");
  if (!reset) throw new Error("Training chat could not be reset");
}

export function trainingChatErrorMessage(error: TrainingChatError): string {
  switch (error.code) {
    case "invalid_request":
      return "Enter a question with no more than 800 characters.";
    case "busy":
      return "Effortline is still working on the previous message.";
    case "model_not_installed":
      return "Install the local model above to start a private training chat.";
    case "model_unavailable":
      return "The local model could not answer. Try again or reinstall it.";
    case "unsafe_model_response":
      return "Effortline could not verify that reply. Try asking in another way.";
    case "library":
      return saveErrorMessage(error.detail);
  }
}
