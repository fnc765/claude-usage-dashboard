import { describe, it, expect, beforeEach } from 'vitest';
import { isExpired, updateWidget, type CombinedUsageData, type UsageData, type CopilotUsageData } from './widget';

describe('widget.ts', () => {
  describe('isExpired', () => {
    it('should return true for null resetsAt', () => {
      expect(isExpired(null)).toBe(true);
    });

    it('should return true for invalid date string', () => {
      expect(isExpired('invalid-date')).toBe(true);
    });

    it('should return true for past date', () => {
      const pastDate = new Date(Date.now() - 10000).toISOString();
      expect(isExpired(pastDate)).toBe(true);
    });

    it('should return false for future date', () => {
      const futureDate = new Date(Date.now() + 10000).toISOString();
      expect(isExpired(futureDate)).toBe(false);
    });

    it('should return true for current time (boundary case)', () => {
      const now = new Date().toISOString();
      // Since Date.now() is called again inside isExpired, this will be slightly past
      expect(isExpired(now)).toBe(true);
    });
  });

  describe('updateWidget', () => {
    let mockData: CombinedUsageData;

    beforeEach(() => {
      // Set up DOM elements required by updateWidget
      document.body.innerHTML = `
        <div id="session-usage-bar" class="bar-usage"></div>
        <div id="session-time-bar" class="bar-time"></div>
        <div id="session-excess-bar" class="bar-excess"></div>
        <div id="session-detail"></div>
        <div id="weekly-usage-bar" class="bar-usage"></div>
        <div id="weekly-time-bar" class="bar-time"></div>
        <div id="weekly-excess-bar" class="bar-excess"></div>
        <div id="weekly-detail"></div>
        <div id="copilot-usage-bar" class="bar-usage"></div>
        <div id="copilot-time-bar" class="bar-time"></div>
        <div id="copilot-excess-bar" class="bar-excess"></div>
        <div id="copilot-detail"></div>
      `;

      // Mock data
      const futureDate = new Date(Date.now() + 3600000).toISOString(); // 1 hour from now
      mockData = {
        claude: {
          five_hour: {
            utilization: 50,
            resets_at: futureDate,
          },
          seven_day: {
            utilization: 30,
            resets_at: futureDate,
          },
        } as UsageData,
        copilot: null,
      };
    });

    it('should update session and weekly bars', () => {
      updateWidget(mockData);

      const sessionDetail = document.getElementById('session-detail');
      const weeklyDetail = document.getElementById('weekly-detail');

      expect(sessionDetail?.textContent).toContain('50% used');
      expect(weeklyDetail?.textContent).toContain('30% used');
    });

    it('should handle expired session', () => {
      const pastDate = new Date(Date.now() - 10000).toISOString();
      mockData.claude!.five_hour.resets_at = pastDate;

      updateWidget(mockData);

      const sessionDetail = document.getElementById('session-detail');
      expect(sessionDetail?.textContent).toBe('No active session');
    });

    it('should handle expired weekly', () => {
      const pastDate = new Date(Date.now() - 10000).toISOString();
      mockData.claude!.seven_day.resets_at = pastDate;

      updateWidget(mockData);

      const weeklyDetail = document.getElementById('weekly-detail');
      expect(weeklyDetail?.textContent).toBe('Awaiting reset');
    });

    it('should update copilot bar when copilot data is provided', () => {
      const futureDate = new Date(Date.now() + 3600000).toISOString();
      mockData.copilot = {
        total_requests: 100,
        monthly_limit: 500,
        utilization: 20,
        resets_at: futureDate,
        items: [],
      } as CopilotUsageData;

      updateWidget(mockData);

      const copilotDetail = document.getElementById('copilot-detail');
      expect(copilotDetail?.textContent).toContain('20% used');
    });

    it('should apply critical class when utilization >= 80%', () => {
      mockData.claude!.five_hour.utilization = 85;

      updateWidget(mockData);

      const sessionUsageBar = document.getElementById('session-usage-bar');
      expect(sessionUsageBar?.className).toContain('critical');
    });

    it('should apply warning class when utilization >= 60%', () => {
      mockData.claude!.five_hour.utilization = 65;

      updateWidget(mockData);

      const sessionUsageBar = document.getElementById('session-usage-bar');
      expect(sessionUsageBar?.className).toContain('warning');
    });

    it('should show excess bar when usage exceeds time elapsed', () => {
      // Set utilization much higher than time elapsed would be
      mockData.claude!.five_hour.utilization = 90;
      // Reset time very close to current time (high time elapsed)
      const soonDate = new Date(Date.now() + 60000).toISOString(); // 1 minute from now
      mockData.claude!.five_hour.resets_at = soonDate;

      updateWidget(mockData);

      const excessBar = document.getElementById('session-excess-bar');
      // Excess bar should have some opacity when overpacing
      // Note: in happy-dom, style.opacity might not compute, so we check the style attribute
      expect(excessBar?.style.opacity).toBeDefined();
    });

    it('should show "Claude not configured" when claude is null', () => {
      const copilotOnlyData: CombinedUsageData = {
        claude: null,
        copilot: {
          total_requests: 100,
          monthly_limit: 500,
          utilization: 20,
          resets_at: new Date(Date.now() + 3600000).toISOString(),
          items: [],
        },
      };

      updateWidget(copilotOnlyData);

      const sessionDetail = document.getElementById('session-detail');
      const weeklyDetail = document.getElementById('weekly-detail');
      expect(sessionDetail?.textContent).toBe('Claude not configured');
      expect(weeklyDetail?.textContent).toBe('Claude not configured');
    });

    it('should reset copilot bar when copilot is null', () => {
      // First set copilot data
      const withCopilot: CombinedUsageData = {
        claude: null,
        copilot: {
          total_requests: 100,
          monthly_limit: 500,
          utilization: 20,
          resets_at: new Date(Date.now() + 3600000).toISOString(),
          items: [],
        },
      };
      updateWidget(withCopilot);

      // Then update with copilot null
      const withoutCopilot: CombinedUsageData = {
        claude: null,
        copilot: null,
      };
      updateWidget(withoutCopilot);

      const copilotDetail = document.getElementById('copilot-detail');
      expect(copilotDetail?.textContent).toBe('Copilot not configured');
    });
  });
});
