// Security: XSS Prevention
// IMPORTANT: Always use textContent instead of innerHTML when displaying data
// Input validation is performed for WSL paths to prevent path traversal attacks

import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_SETTINGS, STORAGE_KEYS, UI_CONSTRAINTS } from "./constants";

export interface Settings {
  opacity: number;
  bgEffect: "transparent" | "mica" | "acrylic";
  alwaysOnTop: boolean;
  pollingInterval: number;
  showClaudeMeters: boolean;
  showCopilotMeter: boolean;
  autostartEnabled: boolean;
}

interface GitHubConfig {
  username: string;
  monthly_limit: number;
}

interface WslConfig {
  credentials_path: string;
}

const STORAGE_KEY = STORAGE_KEYS.WIDGET_SETTINGS;

const DEFAULTS: Settings = {
  opacity: DEFAULT_SETTINGS.OPACITY,
  bgEffect: DEFAULT_SETTINGS.BG_EFFECT,
  alwaysOnTop: DEFAULT_SETTINGS.ALWAYS_ON_TOP,
  pollingInterval: DEFAULT_SETTINGS.POLLING_INTERVAL,
  showClaudeMeters: DEFAULT_SETTINGS.SHOW_CLAUDE_METERS,
  showCopilotMeter: DEFAULT_SETTINGS.SHOW_COPILOT_METER,
  autostartEnabled: DEFAULT_SETTINGS.AUTOSTART_ENABLED,
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

function applyMeterVisibility(settings: Settings): void {
  const sessionMeter = document.querySelector('[data-meter-type="claude-session"]');
  const weeklyMeter = document.querySelector('[data-meter-type="claude-weekly"]');
  const copilotMeter = document.querySelector('[data-meter-type="copilot"]');
  const placeholder = document.getElementById('empty-placeholder');

  if (sessionMeter) sessionMeter.classList.toggle('hidden', !settings.showClaudeMeters);
  if (weeklyMeter) weeklyMeter.classList.toggle('hidden', !settings.showClaudeMeters);
  if (copilotMeter) copilotMeter.classList.toggle('hidden', !settings.showCopilotMeter);

  const allHidden = !settings.showClaudeMeters && !settings.showCopilotMeter;
  if (placeholder) placeholder.style.display = allHidden ? 'flex' : 'none';
}

async function applyAllSettings(settings: Settings): Promise<void> {
  applyOpacity(settings.opacity);
  applyMeterVisibility(settings);

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
      const maxX = window.innerWidth - rect.width - UI_CONSTRAINTS.MENU_PADDING;
      const maxY = window.innerHeight - rect.height - UI_CONSTRAINTS.MENU_PADDING;

      menu.style.left = `${Math.max(UI_CONSTRAINTS.MENU_PADDING, Math.min(x, maxX))}px`;
      menu.style.top = `${Math.max(UI_CONSTRAINTS.MENU_PADDING, Math.min(y, maxY))}px`;
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

    // Sync visibility toggle checkmarks
    const claudeMetersCheck = document.getElementById("claude-meters-check");
    const copilotMeterCheck = document.getElementById("copilot-meter-check");
    if (claudeMetersCheck) {
      claudeMetersCheck.textContent = settings.showClaudeMeters ? "\u2713" : "";
    }
    if (copilotMeterCheck) {
      copilotMeterCheck.textContent = settings.showCopilotMeter ? "\u2713" : "";
    }

    // Autostart checkmark
    const autostartCheck = document.getElementById("autostart-check");
    if (autostartCheck) {
      autostartCheck.textContent = settings.autostartEnabled ? "\u2713" : "";
    }
  }

  applyAllSettings(settings);
  syncMenuUI();

  // WSL セクションは Windows 専用のため、他 OS では非表示にする
  // バックエンドからプラットフォーム情報を取得し、失敗時は userAgent にフォールバック
  applyWslSectionVisibility();

  // Load autostart status from system on startup
  loadAutostartStatus();

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

  // Claude meters visibility toggle
  const toggleClaudeMeters = getEl("toggle-claude-meters");
  const claudeMetersCheck = getEl("claude-meters-check");

  toggleClaudeMeters.addEventListener("click", () => {
    settings.showClaudeMeters = !settings.showClaudeMeters;
    saveSettings(settings);
    claudeMetersCheck.textContent = settings.showClaudeMeters ? "\u2713" : "";
    applyMeterVisibility(settings);
  });

  // Copilot meter visibility toggle
  const toggleCopilotMeter = getEl("toggle-copilot-meter");
  const copilotMeterCheck = getEl("copilot-meter-check");

  toggleCopilotMeter.addEventListener("click", () => {
    settings.showCopilotMeter = !settings.showCopilotMeter;
    saveSettings(settings);
    copilotMeterCheck.textContent = settings.showCopilotMeter ? "\u2713" : "";
    applyMeterVisibility(settings);
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

  // Autostart toggle
  const toggleAutostart = getEl("toggle-autostart");
  const autostartCheck = getEl("autostart-check");

  toggleAutostart.addEventListener("click", async () => {
    try {
      if (settings.autostartEnabled) {
        await invoke("disable_autostart");
        settings.autostartEnabled = false;
      } else {
        await invoke("enable_autostart");
        settings.autostartEnabled = true;
      }
      saveSettings(settings);
      autostartCheck.textContent = settings.autostartEnabled ? "\u2713" : "";
    } catch (e) {
      console.error("Failed to toggle autostart:", e);
      alert(`Failed to toggle autostart: ${e}`);
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
    const monthlyLimit = parseFloat(limitStr) || DEFAULT_SETTINGS.MONTHLY_LIMIT;

    if (!username || !token) {
      alert("Username and Token are required");
      return;
    }

    // トークンの認証確認（GitHub API で検証）
    try {
      await invoke("validate_github_token", { username, token });
    } catch (e) {
      alert(`Token validation failed: ${e}`);
      return;
    }

    // 検証成功後に保存
    try {
      await invoke("save_github_config", {
        username,
        token,
        monthlyLimit,
      });
      alert("GitHub token verified and saved successfully!");
      await invoke("force_refresh");
    } catch (e) {
      alert(`Failed to save settings: ${e}`);
    }
  });

  // WSL 設定の読み込み
  loadWslConfig();

  // WSL 設定の保存
  const saveWslBtn = getEl("save-wsl-config");
  saveWslBtn.addEventListener("click", async () => {
    const credentialsPath = (getEl("wsl-credentials-path") as HTMLInputElement).value.trim();

    if (!credentialsPath) {
      alert("WSL credentials path is required");
      return;
    }

    // セキュリティ検証: WSL UNCパスであることを確認
    if (!credentialsPath.startsWith("\\\\wsl.localhost\\") && !credentialsPath.startsWith("//wsl.localhost/")) {
      alert("Invalid format. Path must start with \\\\wsl.localhost\\\nExample: \\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude\\.credentials.json");
      return;
    }

    // セキュリティ検証: パストラバーサル攻撃を防ぐ
    if (credentialsPath.includes("..")) {
      alert("Invalid path: Path traversal detected");
      return;
    }

    // セキュリティ検証: パスの最大長チェック（DoS対策）
    if (credentialsPath.length > UI_CONSTRAINTS.MAX_PATH_LENGTH) {
      alert(`Path is too long (max ${UI_CONSTRAINTS.MAX_PATH_LENGTH} characters)`);
      return;
    }

    // パスが .credentials.json で終わっていない場合、自動補完
    let normalizedPath = credentialsPath;
    if (!normalizedPath.endsWith('.credentials.json')) {
      if (normalizedPath.endsWith('/') || normalizedPath.endsWith('\\')) {
        normalizedPath += '.credentials.json';
      } else {
        normalizedPath += '/.credentials.json';
      }
      alert(`Path was auto-completed to: ${normalizedPath}`);
    }

    try {
      const result = await invoke<{ warnings: string[] }>("save_wsl_config", {
        credentialsPath: normalizedPath,
      });
      if (result.warnings.length > 0) {
        alert(
          "WSL path saved with warnings:\n" + result.warnings.join("\n"),
        );
      } else {
        alert("WSL settings saved successfully!");
      }
      await invoke("force_refresh");
    } catch (e) {
      alert(`Failed to save WSL settings: ${e}`);
    }
  });

  // WSL 設定のクリア
  const clearWslBtn = getEl("clear-wsl-config");
  clearWslBtn.addEventListener("click", async () => {
    try {
      await invoke("clear_wsl_config");
      (getEl("wsl-credentials-path") as HTMLInputElement).value = "";
      alert("WSL settings cleared successfully!");
      await invoke("force_refresh");
    } catch (e) {
      alert(`Failed to clear WSL settings: ${e}`);
    }
  });
}

async function loadGitHubConfig() {
  try {
    const config = await invoke<GitHubConfig>("get_github_config");
    if (config) {
      const usernameEl = document.getElementById("github-username") as HTMLInputElement;
      const limitEl = document.getElementById("monthly-limit") as HTMLInputElement;
      if (usernameEl) usernameEl.value = config.username || "";
      if (limitEl) limitEl.value = String(config.monthly_limit || DEFAULT_SETTINGS.MONTHLY_LIMIT);
      // トークンは表示しない（セキュリティ上の理由）
    }
  } catch (e) {
    console.error("Failed to load GitHub config:", e);
  }
}

async function loadWslConfig() {
  try {
    const config = await invoke<WslConfig>("get_wsl_config");
    if (config) {
      const pathEl = document.getElementById("wsl-credentials-path") as HTMLInputElement;
      if (pathEl) pathEl.value = config.credentials_path || "";
    }
  } catch (e) {
    console.error("Failed to load WSL config:", e);
  }
}

async function loadAutostartStatus() {
  try {
    const isEnabled = await invoke("is_autostart_enabled") as boolean;
    const settings = loadSettings();
    settings.autostartEnabled = isEnabled;
    saveSettings(settings);

    const autostartCheck = document.getElementById("autostart-check");
    if (autostartCheck) {
      autostartCheck.textContent = isEnabled ? "\u2713" : "";
    }
  } catch (e) {
    console.error("Failed to load autostart status:", e);
  }
}

interface PlatformInfo {
  os: string;
  is_wsl_supported: boolean;
}

/** Determine WSL section visibility using backend platform info, with userAgent fallback. */
async function applyWslSectionVisibility(): Promise<void> {
  let isWslSupported: boolean;
  try {
    const platformInfo = await invoke<PlatformInfo>("get_platform_info");
    isWslSupported = platformInfo.is_wsl_supported;
  } catch {
    // Fallback: use navigator.userAgent when backend is unavailable
    isWslSupported = navigator.userAgent.toLowerCase().includes("windows");
  }

  if (!isWslSupported) {
    const wslSection = document.getElementById("wsl-section");
    const wslDivider = document.getElementById("wsl-divider");
    if (wslSection) wslSection.style.display = "none";
    if (wslDivider) wslDivider.style.display = "none";
  }
}
