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

function getEl(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Required DOM element #${id} not found`);
  return el;
}

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

export function initContextMenu(): void {
  const settings = loadSettings();

  // Cache DOM elements
  const menu = getEl("context-menu");
  const opacitySlider = getEl("opacity-slider") as HTMLInputElement;
  const opacityValue = getEl("opacity-value");
  const aotCheck = getEl("aot-check");
  const toggleAot = getEl("toggle-aot");
  const forceRefresh = getEl("force-refresh");
  const quitApp = getEl("quit-app");

  function showMenu(x: number, y: number): void {
    menu.style.visibility = "hidden";
    menu.style.display = "block";

    requestAnimationFrame(() => {
      const rect = menu.getBoundingClientRect();
      const maxX = window.innerWidth - rect.width - 4;
      const maxY = window.innerHeight - rect.height - 4;

      menu.style.left = `${Math.max(4, Math.min(x, maxX))}px`;
      menu.style.top = `${Math.max(4, Math.min(y, maxY))}px`;
      menu.style.visibility = "visible";
    });
  }

  function hideMenu(): void {
    menu.style.display = "none";
  }

  function isMenuVisible(): boolean {
    return menu.style.display !== "none";
  }

  function syncMenuUI(): void {
    opacitySlider.value = String(settings.opacity);
    opacityValue.textContent = `${settings.opacity}%`;

    document.querySelectorAll<HTMLElement>("[data-effect]").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.effect === settings.bgEffect);
    });

    aotCheck.textContent = settings.alwaysOnTop ? "\u2713" : "";

    document.querySelectorAll<HTMLElement>("[data-interval]").forEach((btn) => {
      btn.classList.toggle(
        "active",
        parseInt(btn.dataset.interval!) === settings.pollingInterval,
      );
    });
  }

  applyAllSettings(settings);
  syncMenuUI();

  // Right-click to open
  document.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (isMenuVisible()) {
      hideMenu();
    } else {
      syncMenuUI();
      showMenu(e.clientX, e.clientY);
    }
  });

  // Click outside to close
  document.addEventListener("mousedown", (e) => {
    if (isMenuVisible() && !menu.contains(e.target as Node)) {
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
  opacitySlider.addEventListener("input", () => {
    const val = parseInt(opacitySlider.value);
    opacityValue.textContent = `${val}%`;
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
      try {
        await invoke("set_background_effect", { effect });
      } catch (e) {
        console.warn("Failed to set background effect:", e);
      }
    });
  });

  // Always on top toggle
  toggleAot.addEventListener("click", async () => {
    settings.alwaysOnTop = !settings.alwaysOnTop;
    saveSettings(settings);
    aotCheck.textContent = settings.alwaysOnTop ? "\u2713" : "";
    try {
      await invoke("set_always_on_top", { enabled: settings.alwaysOnTop });
    } catch (e) {
      console.warn("Failed to set always on top:", e);
    }
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
      try {
        await invoke("set_polling_interval", { seconds });
      } catch (e) {
        console.warn("Failed to set polling interval:", e);
      }
    });
  });

  // Force refresh
  forceRefresh.addEventListener("click", async () => {
    try {
      await invoke("force_refresh");
    } catch (e) {
      console.warn("Failed to force refresh:", e);
    }
    hideMenu();
  });

  // Quit
  quitApp.addEventListener("click", async () => {
    try {
      await invoke("quit_app");
    } catch (e) {
      console.warn("Failed to quit app:", e);
    }
  });

  // GitHub 設定の読み込み
  loadGitHubConfig();

  // GitHub 設定の保存
  const saveBtn = getEl("save-github-config");
  saveBtn.addEventListener("click", async () => {
    const username = (getEl("github-username") as HTMLInputElement).value.trim();
    const token = (getEl("github-token") as HTMLInputElement).value.trim();
    const limitStr = (getEl("monthly-limit") as HTMLInputElement).value.trim();
    const monthlyLimit = parseFloat(limitStr) || 300;

    if (!username || !token) {
      alert("Username and Token are required");
      return;
    }

    try {
      await invoke("save_github_config", {
        username,
        token,
        monthlyLimit,
      });
      alert("Settings saved successfully!");
      await invoke("force_refresh");
    } catch (e) {
      alert(`Failed to save settings: ${e}`);
    }
  });
}

async function loadGitHubConfig() {
  try {
    const config = await invoke("get_github_config") as any;
    if (config) {
      const usernameEl = document.getElementById("github-username") as HTMLInputElement;
      const limitEl = document.getElementById("monthly-limit") as HTMLInputElement;
      if (usernameEl) usernameEl.value = config.username || "";
      if (limitEl) limitEl.value = String(config.monthly_limit || 300);
      // トークンは表示しない（セキュリティ上の理由）
    }
  } catch (e) {
    console.error("Failed to load GitHub config:", e);
  }
}
