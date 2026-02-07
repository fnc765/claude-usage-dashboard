export interface UsageData {
  five_hour: { utilization: number; resets_at: string };
  seven_day: { utilization: number; resets_at: string };
}

interface BarElements {
  usageBar: HTMLElement;
  timeBar: HTMLElement;
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
) {
  const cls = getThresholdClass(usagePercent);

  elements.usageBar.style.width = `${usagePercent}%`;
  elements.timeBar.style.width = `${timePercent}%`;

  elements.usageBar.className = "bar-usage" + (cls ? ` ${cls}` : "");
  elements.timeBar.className = "bar-time" + (cls ? ` ${cls}` : "");

  const resetTimeStr = formatResetTime(resetsAt);
  const remainStr = formatRemaining(resetsAt);
  elements.detail.textContent =
    `${Math.round(usagePercent)}% used  ·  Reset ${resetTimeStr} (${remainStr})`;
}

export function updateWidget(data: UsageData) {
  const sessionElements: BarElements = {
    usageBar: document.getElementById("session-usage-bar")!,
    timeBar: document.getElementById("session-time-bar")!,
    detail: document.getElementById("session-detail")!,
  };

  const weeklyElements: BarElements = {
    usageBar: document.getElementById("weekly-usage-bar")!,
    timeBar: document.getElementById("weekly-time-bar")!,
    detail: document.getElementById("weekly-detail")!,
  };

  const sessionTimePercent = calcTimeElapsedPercent(data.five_hour.resets_at, 5);
  const weeklyTimePercent = calcTimeElapsedPercent(data.seven_day.resets_at, 168);

  updateBar(
    sessionElements,
    data.five_hour.utilization,
    sessionTimePercent,
    data.five_hour.resets_at,
  );

  updateBar(
    weeklyElements,
    data.seven_day.utilization,
    weeklyTimePercent,
    data.seven_day.resets_at,
  );
}
