import { invoke } from "@tauri-apps/api/core";

export async function invokeCommand(command, payload = {}) {
  const response = await invoke(command, payload);
  if (!response) {
    throw new Error("Empty response");
  }
  if (typeof response.code === "number" && response.code !== 0) {
    throw new Error(response.message || "Request failed");
  }
  return response.data;
}
