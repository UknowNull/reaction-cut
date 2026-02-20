import { message as dialogMessage } from "@tauri-apps/plugin-dialog";

export async function showErrorDialog(error, title = "提交失败") {
  const text =
    typeof error === "string"
      ? error
      : error?.message || String(error || "请求失败");
  await dialogMessage(text, { title });
}
