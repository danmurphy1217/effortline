import { Channel, invoke } from "@tauri-apps/api/core";

export type LocalModelStatus = {
  version: number;
  installed: boolean;
  installing: boolean;
  download_bytes: number;
  required_storage_bytes: number;
};

export type LocalModelProgress = {
  downloaded_bytes: number;
  total_bytes: number;
  stage: "downloading" | "verifying" | "installed" | "cancelled" | "failed";
};

export async function getLocalModelStatus(): Promise<LocalModelStatus> {
  return invoke("local_model_status");
}

export async function installLocalModel(
  onProgress: (progress: LocalModelProgress) => void,
): Promise<LocalModelStatus> {
  const progress = new Channel<LocalModelProgress>();
  progress.onmessage = onProgress;
  return invoke("install_local_model", { progress });
}

export async function cancelLocalModelInstall(): Promise<boolean> {
  return invoke("cancel_local_model_install");
}

export async function removeLocalModel(): Promise<void> {
  return invoke("remove_local_model");
}

export function modelErrorMessage(error: unknown): string {
  const code = typeof error === "string" ? error : "";
  if (code.includes("verification_failed"))
    return "Effortline could not verify the model. Remove it and install it again.";
  if (code.includes("download_failed"))
    return "The model download stopped. Check your connection and try again.";
  if (code.includes("cancelled"))
    return "The model download was cancelled. You can try again.";
  if (code.includes("install_in_progress"))
    return "A model task is already running. Wait for it to finish.";
  if (code.includes("removal_failed"))
    return "Effortline could not remove the model. Try again.";
  if (code.includes("model_load_failed"))
    return "Effortline could not load the local model. Remove it and install it again.";
  if (code.includes("inference_failed"))
    return "The local model returned an explanation Effortline could not check. The measured result is still available.";
  if (code.includes("location_unavailable"))
    return "Effortline could not find a safe place for the model on this Mac.";
  return "Effortline could not complete that model task. Try again.";
}
