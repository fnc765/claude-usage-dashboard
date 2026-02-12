export interface UsageMeter {
  utilization: number;
  resets_at: string | null;
}

export interface ExtraUsage {
  is_enabled: boolean;
  monthly_limit: number;
  used_credits: number;
  utilization: number;
}

export interface UsageData {
  five_hour: UsageMeter;
  seven_day: UsageMeter;
  seven_day_oauth_apps?: UsageMeter | null;
  seven_day_opus?: UsageMeter | null;
  seven_day_sonnet?: UsageMeter | null;
  seven_day_cowork?: UsageMeter | null;
  iguana_necktie?: unknown;
  extra_usage?: ExtraUsage | null;
}

export interface CopilotUsageItem {
  model: string;
  gross_quantity: number;
}

export interface CopilotUsageData {
  total_requests: number;
  monthly_limit: number;
  utilization: number;
  resets_at: string;
  items: CopilotUsageItem[];
}

export interface CombinedUsageData {
  claude: UsageData;
  copilot?: CopilotUsageData | null;
}

interface BarElements {
  usageBar: HTMLElement;
  timeBar: HTMLElement;
  excessBar: HTMLElement;
  detail: HTMLElement;
}

function getThresholdClass(percent: number): string {
  if (percent >= 80) return "critical";
  if (percent >= 60) return "warning";
  return "";
}

function formatRemaining(resetsAt: string | null): string {
  if (!resetsAt) return "";
  const reset = new Date(resetsAt);
  if (isNaN(reset.getTime())) return "";
  const now = new Date();
  const diffMs = reset.getTime() - now.getTime();

  if (diffMs <= 0) return "resetting...";

  const totalMin = Math.floor(diffMs / 60000);
  const days = Math.floor(totalMin / 1440);
  const hours = Math.floor((totalMin % 1440) / 60);
  const minutes = totalMin % 60;

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function formatResetTime(resetsAt: string | null): string {
  if (!resetsAt) return "";
  const reset = new Date(resetsAt);
  if (isNaN(reset.getTime())) return "";
  const now = new Date();
  const diffMs = reset.getTime() - now.getTime();

  if (diffMs <= 0) return "resetting...";

  const totalHours = diffMs / 3600000;
  if (totalHours < 24) {
    return reset.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
  }
  return reset.toLocaleDateString("ja-JP", { month: "numeric", day: "numeric" });
}

export function isExpired(resetsAt: string | null): boolean {
  if (!resetsAt) return true;
  const time = new Date(resetsAt).getTime();
  if (isNaN(time)) return true;
  return time - Date.now() <= 0;
}

function calcTimeElapsedPercent(resetsAt: string | null, windowHours: number): number {
  if (!resetsAt) return 0;
  const reset = new Date(resetsAt);
  if (isNaN(reset.getTime())) return 0;
  const now = new Date();
  const remainMs = reset.getTime() - now.getTime();
  const totalMs = windowHours * 3600000;
  const elapsed = totalMs - remainMs;
  return Math.max(0, Math.min(100, (elapsed / totalMs) * 100));
}

function calcMonthlyTimeElapsedPercent(resetsAt: string | null): number {
  if (!resetsAt) return 0;
  const reset = new Date(resetsAt);
  if (isNaN(reset.getTime())) return 0;

  const now = new Date();
  const monthStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1, 0, 0, 0)
  );

  const totalMs = reset.getTime() - monthStart.getTime();
  const elapsedMs = now.getTime() - monthStart.getTime();

  return Math.max(0, Math.min(100, (elapsedMs / totalMs) * 100));
}

function updateBar(
  elements: BarElements,
  usagePercent: number,
  timePercent: number,
  resetsAt: string | null,
  inactiveLabel: string,
) {
  if (isExpired(resetsAt)) {
    elements.usageBar.style.width = "0%";
    elements.timeBar.style.width = "0%";
    elements.excessBar.style.width = "0%";
    elements.excessBar.style.left = "0%";
    elements.excessBar.style.opacity = "0";
    elements.usageBar.className = "bar-usage";
    elements.timeBar.className = "bar-time";
    elements.excessBar.className = "bar-excess";
    elements.detail.textContent = inactiveLabel;
    return;
  }

  const cls = getThresholdClass(usagePercent);
  const isOverpace = usagePercent > timePercent;

  elements.timeBar.style.width = `${timePercent}%`;
  elements.timeBar.className = "bar-time" + (cls ? ` ${cls}` : "");

  if (isOverpace) {
    elements.usageBar.style.width = `${timePercent}%`;
    elements.usageBar.style.borderRadius = "6px 0 0 6px";

    const excessWidth = usagePercent - timePercent;
    elements.excessBar.style.left = `${timePercent}%`;
    elements.excessBar.style.width = `${excessWidth}%`;
    elements.excessBar.style.opacity = "1";
    elements.excessBar.className = "bar-excess" + (cls ? ` ${cls}` : "");
  } else {
    elements.usageBar.style.width = `${usagePercent}%`;
    elements.usageBar.style.borderRadius = "";

    elements.excessBar.style.width = "0%";
    elements.excessBar.style.left = `${usagePercent}%`;
    elements.excessBar.style.opacity = "0";
    elements.excessBar.className = "bar-excess";
  }

  elements.usageBar.className = "bar-usage" + (cls ? ` ${cls}` : "");

  const resetTimeStr = formatResetTime(resetsAt);
  const remainStr = formatRemaining(resetsAt);
  const excessSuffix = isOverpace
    ? ` (+${Math.round(usagePercent - timePercent)}%)`
    : "";
  elements.detail.textContent =
    `${Math.round(usagePercent)}% used${excessSuffix}  ·  Reset ${resetTimeStr} (${remainStr})`;
}

function getElement(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Required DOM element #${id} not found`);
  return el;
}

function updateCopilotBar(copilot: CopilotUsageData) {
  const copilotElements: BarElements = {
    usageBar: getElement("copilot-usage-bar"),
    timeBar: getElement("copilot-time-bar"),
    excessBar: getElement("copilot-excess-bar"),
    detail: getElement("copilot-detail"),
  };

  const timePercent = calcMonthlyTimeElapsedPercent(copilot.resets_at);

  updateBar(
    copilotElements,
    copilot.utilization,
    timePercent,
    copilot.resets_at,
    "Not configured",
  );
}

export function updateWidget(data: CombinedUsageData) {
  const sessionElements: BarElements = {
    usageBar: getElement("session-usage-bar"),
    timeBar: getElement("session-time-bar"),
    excessBar: getElement("session-excess-bar"),
    detail: getElement("session-detail"),
  };

  const weeklyElements: BarElements = {
    usageBar: getElement("weekly-usage-bar"),
    timeBar: getElement("weekly-time-bar"),
    excessBar: getElement("weekly-excess-bar"),
    detail: getElement("weekly-detail"),
  };

  const sessionTimePercent = calcTimeElapsedPercent(data.claude.five_hour.resets_at, 5);
  const weeklyTimePercent = calcTimeElapsedPercent(data.claude.seven_day.resets_at, 168);

  updateBar(
    sessionElements,
    data.claude.five_hour.utilization,
    sessionTimePercent,
    data.claude.five_hour.resets_at,
    "No active session",
  );

  updateBar(
    weeklyElements,
    data.claude.seven_day.utilization,
    weeklyTimePercent,
    data.claude.seven_day.resets_at,
    "Awaiting reset",
  );

  // Copilot 使用量更新
  if (data.copilot) {
    updateCopilotBar(data.copilot);
  }
}
