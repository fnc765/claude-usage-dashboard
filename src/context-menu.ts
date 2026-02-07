import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  opacity: number;
  bgEffect: "transparent" | "mica" | "acrylic";
  alwaysOnTop: boolean;
  pollingInterval: number;
}

const STORAGE_KEY = "widget-settings";

const DEFAULTS: Settings = {
  opacity: 75,
  bgEffect: "mica",
  alwaysOnTop: true,
  pollingInterval: 60,
};

function loadSettings(): Settings {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (raw) {
    try {
      return { ...DEFAULTS, ...JSON.parse(raw) };
    } catch {
      return { ...DEFAULTS };
    }
  }
  return { ...DEFAULTS };
}

function saveSettings(settings: Settings): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

function applyOpacity(opacity: number): void {
  const widget = document.querySelector(".widget") as HTMLElement;
  if (widget) {
    widget.style.background = `rgba(18, 18, 18, ${opacity / 100})`;
  }
}

async function applyAllSettings(settings: Settings): Promise<void> {
  applyOpacity(settings.opacity);

  try {
    await invoke("set_background_effect", { effect: settings.bgEffect });
  } catch (e) {
    console.warn("Failed to set background effect:", e);
  }

  try {
    await invoke("set_always_on_top", { enabled: settings.alwaysOnTop });
  } catch (e) {
    console.warn("Failed to set always on top:", e);
  }

  try {
    await invoke("set_polling_interval", { seconds: settings.pollingInterval });
  } catch (e) {
    console.warn("Failed to set polling interval:", e);
  }
}

function showMenu(x: number, y: number): void {
  const menu = document.getElementById("context-menu")!;
  menu.style.display = "block";

  requestAnimationFrame(() => {
    const rect = menu.getBoundingClientRect();
    const maxX = window.innerWidth - rect.width - 4;
    const maxY = window.innerHeight - rect.height - 4;

    menu.style.left = `${Math.max(4, Math.min(x, maxX))}px`;
    menu.style.top = `${Math.max(4, Math.min(y, maxY))}px`;
  });
}

function hideMenu(): void {
  document.getElementById("context-menu")!.style.display = "none";
}

function isMenuVisible(): boolean {
  return document.getElementById("context-menu")!.style.display !== "none";
}

function syncMenuUI(settings: Settings): void {
  const slider = document.getElementById("opacity-slider") as HTMLInputElement;
  slider.value = String(settings.opacity);
  document.getElementById("opacity-value")!.textContent = `${settings.opacity}%`;

  document.querySelectorAll<HTMLElement>("[data-effect]").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.effect === settings.bgEffect);
  });

  document.getElementById("aot-check")!.textContent = settings.alwaysOnTop
    ? "\u2713"
    : "";

  document.querySelectorAll<HTMLElement>("[data-interval]").forEach((btn) => {
    btn.classList.toggle(
      "active",
      parseInt(btn.dataset.interval!) === settings.pollingInterval,
    );
  });
}

export function initContextMenu(): void {
  const settings = loadSettings();

  applyAllSettings(settings);
  syncMenuUI(settings);

  // Right-click to open
  document.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (isMenuVisible()) {
      hideMenu();
    } else {
      syncMenuUI(settings);
      showMenu(e.clientX, e.clientY);
    }
  });

  // Click outside to close
  document.addEventListener("mousedown", (e) => {
    if (
      isMenuVisible() &&
      !document.getElementById("context-menu")!.contains(e.target as Node)
    ) {
      hideMenu();
    }
  });

  // Escape key to close
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && isMenuVisible()) {
      hideMenu();
    }
  });

  // Opacity slider
  const opacitySlider = document.getElementById(
    "opacity-slider",
  ) as HTMLInputElement;
  opacitySlider.addEventListener("input", () => {
    const val = parseInt(opacitySlider.value);
    document.getElementById("opacity-value")!.textContent = `${val}%`;
    applyOpacity(val);
    settings.opacity = val;
    saveSettings(settings);
  });

  // Background effect buttons
  document.querySelectorAll<HTMLElement>("[data-effect]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const effect = btn.dataset.effect as Settings["bgEffect"];
      document
        .querySelectorAll("[data-effect]")
        .forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      settings.bgEffect = effect;
      saveSettings(settings);
      await invoke("set_background_effect", { effect });
    });
  });

  // Always on top toggle
  document.getElementById("toggle-aot")!.addEventListener("click", async () => {
    settings.alwaysOnTop = !settings.alwaysOnTop;
    saveSettings(settings);
    document.getElementById("aot-check")!.textContent = settings.alwaysOnTop
      ? "\u2713"
      : "";
    await invoke("set_always_on_top", { enabled: settings.alwaysOnTop });
  });

  // Polling interval buttons
  document.querySelectorAll<HTMLElement>("[data-interval]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const seconds = parseInt(btn.dataset.interval!);
      document
        .querySelectorAll("[data-interval]")
        .forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      settings.pollingInterval = seconds;
      saveSettings(settings);
      await invoke("set_polling_interval", { seconds });
    });
  });

  // Force refresh
  document
    .getElementById("force-refresh")!
    .addEventListener("click", async () => {
      await invoke("force_refresh");
      hideMenu();
    });

  // Quit
  document.getElementById("quit-app")!.addEventListener("click", async () => {
    await invoke("quit_app");
  });
}
