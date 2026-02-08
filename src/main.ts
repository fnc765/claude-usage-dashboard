import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { updateWidget, isExpired, type UsageData } from "./widget";
import { initContextMenu } from "./context-menu";

let latestData: UsageData | null = null;
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
    const data = await invoke<UsageData>("get_usage");
    latestData = data;
    refreshTriggered = false;
    updateWidget(data);
  } catch {
    // Will be updated via events once API connects
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  initDrag();
  initContextMenu();

  await listen<UsageData>("usage-update", (event) => {
    latestData = event.payload;
    refreshTriggered = false;
    updateWidget(event.payload);
  });

  await fetchInitialData();

  setInterval(() => {
    if (!latestData) return;

    updateWidget(latestData);

    const sessionExpired = isExpired(latestData.five_hour.resets_at);
    const weeklyExpired = isExpired(latestData.seven_day.resets_at);

    if ((sessionExpired || weeklyExpired) && !refreshTriggered) {
      refreshTriggered = true;
      invoke("force_refresh").catch(() => {
        refreshTriggered = false;
      });
    }
  }, 10_000);
});
