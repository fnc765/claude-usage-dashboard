import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { updateWidget, isExpired, type CombinedUsageData, type CopilotUsageData } from "./widget";
import { initContextMenu } from "./context-menu";

let latestData: CombinedUsageData | null = null;
let refreshTriggered = false;

async function initDrag() {
  const dragRegion = document.querySelector("[data-tauri-drag-region]");
  if (dragRegion) {
    dragRegion.addEventListener("mousedown", async (e) => {
      const mouseEvent = e as MouseEvent;
      if (mouseEvent.button !== 0) return;
      const target = mouseEvent.target as HTMLElement;
      if (target.closest(".bar-track") || target.closest("button")) return;
      await getCurrentWindow().startDragging();
    });
  }
}

async function fetchInitialData() {
  try {
    const data = await invoke<CombinedUsageData>("get_usage");
    // get_usage returns only Claude data, wrap it in CombinedUsageData format
    if (data && "five_hour" in data) {
      latestData = { claude: data as any, copilot: null };
    } else {
      latestData = data;
    }
    refreshTriggered = false;
    if (latestData) updateWidget(latestData);
  } catch {
    // Will be updated via events once API connects
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  initDrag();
  initContextMenu();

  await listen<CombinedUsageData>("usage-update", (event) => {
    latestData = event.payload;
    refreshTriggered = false;
    updateWidget(event.payload);
  });

  await listen<CopilotUsageData>("copilot-only-update", (event) => {
    if (latestData) {
      latestData.copilot = event.payload;
      updateWidget(latestData);
    }
  });

  await listen<string>("token-status", (event) => {
    const statusEl = document.getElementById("token-status");
    if (!statusEl) return;

    switch (event.payload) {
      case "expired":
        statusEl.textContent = "⚠ Token expired";
        statusEl.className = "token-status error";
        statusEl.title = "アクセストークンの有効期限が切れました。\nターミナルで claude コマンドを実行すると更新されます。";
        break;
      case "error":
        statusEl.textContent = "⚠ No credentials";
        statusEl.className = "token-status error";
        statusEl.title = "~/.claude/.credentials.json が見つかりません。\nターミナルで claude login を実行してください。";
        break;
      case "fetch_error":
        statusEl.textContent = "⚠ Fetch error";
        statusEl.className = "token-status warning";
        statusEl.title = "API からデータを取得できませんでした。\nネットワーク接続を確認してください。";
        break;
      case "ok":
        statusEl.textContent = "";
        statusEl.className = "token-status";
        statusEl.title = "";
        break;
    }
  });

  await fetchInitialData();

  setInterval(() => {
    if (!latestData) return;

    updateWidget(latestData);

    const sessionExpired = isExpired(latestData.claude.five_hour.resets_at);
    const weeklyExpired = isExpired(latestData.claude.seven_day.resets_at);
    const copilotExpired = latestData.copilot
      ? isExpired(latestData.copilot.resets_at)
      : false;

    if ((sessionExpired || weeklyExpired || copilotExpired) && !refreshTriggered) {
      refreshTriggered = true;
      invoke("force_refresh").catch(() => {
        refreshTriggered = false;
      });
    }
  }, 10_000);
});
