export interface UsageData {
  five_hour: { utilization: number; resets_at: string };
  seven_day: { utilization: number; resets_at: string };
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

function formatRemaining(resetsAt: string): string {
  const reset = new Date(resetsAt);
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

function formatResetTime(resetsAt: string): string {
  const reset = new Date(resetsAt);
  const now = new Date();
  const diffMs = reset.getTime() - now.getTime();

  if (diffMs <= 0) return "resetting...";

  const totalHours = diffMs / 3600000;
  if (totalHours < 24) {
    return reset.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
  }
  return reset.toLocaleDateString("ja-JP", { month: "numeric", day: "numeric" });
}

export function isExpired(resetsAt: string): boolean {
  return new Date(resetsAt).getTime() - Date.now() <= 0;
}

function calcTimeElapsedPercent(resetsAt: string, windowHours: number): number {
  const reset = new Date(resetsAt);
  const now = new Date();
  const remainMs = reset.getTime() - now.getTime();
  const totalMs = windowHours * 3600000;
  const elapsed = totalMs - remainMs;
  return Math.max(0, Math.min(100, (elapsed / totalMs) * 100));
}

function updateBar(
  elements: BarElements,
  usagePercent: number,
  timePercent: number,
  resetsAt: string,
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

export function updateWidget(data: UsageData) {
  const sessionElements: BarElements = {
    usageBar: document.getElementById("session-usage-bar")!,
    timeBar: document.getElementById("session-time-bar")!,
    excessBar: document.getElementById("session-excess-bar")!,
    detail: document.getElementById("session-detail")!,
  };

  const weeklyElements: BarElements = {
    usageBar: document.getElementById("weekly-usage-bar")!,
    timeBar: document.getElementById("weekly-time-bar")!,
    excessBar: document.getElementById("weekly-excess-bar")!,
    detail: document.getElementById("weekly-detail")!,
  };

  const sessionTimePercent = calcTimeElapsedPercent(data.five_hour.resets_at, 5);
  const weeklyTimePercent = calcTimeElapsedPercent(data.seven_day.resets_at, 168);

  updateBar(
    sessionElements,
    data.five_hour.utilization,
    sessionTimePercent,
    data.five_hour.resets_at,
    "No active session",
  );

  updateBar(
    weeklyElements,
    data.seven_day.utilization,
    weeklyTimePercent,
    data.seven_day.resets_at,
    "Awaiting reset",
  );
}
