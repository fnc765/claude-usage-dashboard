import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { updateWidget, type UsageData } from "./widget";

async function initDrag() {
  const dragRegion = document.querySelector("[data-tauri-drag-region]");
  if (dragRegion) {
    dragRegion.addEventListener("mousedown", async (e) => {
      const target = e.target as HTMLElement;
      if (target.closest(".bar-track") || target.closest("button")) return;
      await getCurrentWindow().startDragging();
    });
  }
}

async function fetchInitialData() {
  try {
    const data = await invoke<UsageData>("get_usage");
    updateWidget(data);
  } catch {
    // Will be updated via events once API connects
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  initDrag();

  await listen<UsageData>("usage-update", (event) => {
    updateWidget(event.payload);
  });

  await fetchInitialData();
});
