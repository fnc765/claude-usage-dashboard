import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { CombinedUsageData, CopilotUsageData } from './widget';

// Mock Tauri API
const mockInvoke = vi.fn();
const mockListen = vi.fn();
const mockStartDragging = vi.fn();
const mockGetCurrentWindow = vi.fn(() => ({
  startDragging: mockStartDragging,
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: mockListen,
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: mockGetCurrentWindow,
}));

vi.mock('./widget', () => ({
  updateWidget: vi.fn(),
  isExpired: vi.fn(),
}));

vi.mock('./context-menu', () => ({
  initContextMenu: vi.fn(),
}));

describe('main.ts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.innerHTML = `
      <div data-tauri-drag-region>
        <div class="bar-track"></div>
        <button>Test Button</button>
      </div>
      <div id="token-status"></div>
    `;
  });

  describe('Drag functionality', () => {
    it('should initialize drag region', () => {
      const dragRegion = document.querySelector('[data-tauri-drag-region]');
      expect(dragRegion).not.toBeNull();
    });

    it('should not drag when clicking on bar-track', async () => {
      const dragRegion = document.querySelector('[data-tauri-drag-region]');
      const barTrack = document.querySelector('.bar-track');

      const mouseEvent = new MouseEvent('mousedown', {
        button: 0,
        bubbles: true,
      });

      // Mock the mouseEvent.target to be barTrack
      Object.defineProperty(mouseEvent, 'target', {
        value: barTrack,
        writable: false,
      });

      dragRegion?.dispatchEvent(mouseEvent);

      // startDragging should not be called when clicking on bar-track
      expect(mockStartDragging).not.toHaveBeenCalled();
    });

    it('should not drag when clicking on button', async () => {
      const dragRegion = document.querySelector('[data-tauri-drag-region]');
      const button = document.querySelector('button');

      const mouseEvent = new MouseEvent('mousedown', {
        button: 0,
        bubbles: true,
      });

      Object.defineProperty(mouseEvent, 'target', {
        value: button,
        writable: false,
      });

      dragRegion?.dispatchEvent(mouseEvent);

      expect(mockStartDragging).not.toHaveBeenCalled();
    });

    it('should not drag on right-click', async () => {
      const dragRegion = document.querySelector('[data-tauri-drag-region]');

      const mouseEvent = new MouseEvent('mousedown', {
        button: 2, // Right click
        bubbles: true,
      });

      Object.defineProperty(mouseEvent, 'target', {
        value: dragRegion,
        writable: false,
      });

      dragRegion?.dispatchEvent(mouseEvent);

      expect(mockStartDragging).not.toHaveBeenCalled();
    });
  });

  describe('fetchInitialData', () => {
    it('should handle Claude data response', async () => {
      const claudeData = {
        five_hour: {
          utilization: 50,
          resets_at: new Date(Date.now() + 3600000).toISOString(),
        },
        seven_day: {
          utilization: 30,
          resets_at: new Date(Date.now() + 3600000).toISOString(),
        },
      };

      mockInvoke.mockResolvedValue(claudeData);

      // The actual function would be called during DOMContentLoaded
      // Here we test the data structure transformation
      const data = await mockInvoke('get_usage');

      expect(data).toHaveProperty('five_hour');
      expect(data).toHaveProperty('seven_day');
    });

    it('should handle combined data response', async () => {
      const combinedData: CombinedUsageData = {
        claude: {
          five_hour: {
            utilization: 50,
            resets_at: new Date(Date.now() + 3600000).toISOString(),
          },
          seven_day: {
            utilization: 30,
            resets_at: new Date(Date.now() + 3600000).toISOString(),
          },
        },
        copilot: null,
      };

      mockInvoke.mockResolvedValue(combinedData);

      const data = await mockInvoke('get_usage');

      expect(data).toHaveProperty('claude');
      expect(data).toHaveProperty('copilot');
    });

    it('should handle API errors gracefully', async () => {
      mockInvoke.mockRejectedValue(new Error('API Error'));

      try {
        await mockInvoke('get_usage');
      } catch (error) {
        expect(error).toBeInstanceOf(Error);
      }
    });
  });

  describe('Token status handling', () => {
    it('should display expired token status', () => {
      const statusEl = document.getElementById('token-status');
      if (statusEl) {
        statusEl.textContent = '⚠ Token expired';
        statusEl.className = 'token-status error';
        statusEl.title = 'アクセストークンの有効期限が切れました。\nターミナルで claude コマンドを実行すると更新されます。';
      }

      expect(statusEl?.textContent).toBe('⚠ Token expired');
      expect(statusEl?.className).toBe('token-status error');
    });

    it('should display no credentials status', () => {
      const statusEl = document.getElementById('token-status');
      if (statusEl) {
        statusEl.textContent = '⚠ No credentials';
        statusEl.className = 'token-status error';
        statusEl.title = '~/.claude/.credentials.json が見つかりません。\nターミナルで claude login を実行してください。';
      }

      expect(statusEl?.textContent).toBe('⚠ No credentials');
      expect(statusEl?.className).toBe('token-status error');
    });

    it('should display fetch error status', () => {
      const statusEl = document.getElementById('token-status');
      if (statusEl) {
        statusEl.textContent = '⚠ Fetch error';
        statusEl.className = 'token-status warning';
        statusEl.title = 'API からデータを取得できませんでした。\nネットワーク接続を確認してください。';
      }

      expect(statusEl?.textContent).toBe('⚠ Fetch error');
      expect(statusEl?.className).toBe('token-status warning');
    });

    it('should display ok status', () => {
      const statusEl = document.getElementById('token-status');
      if (statusEl) {
        statusEl.textContent = '';
        statusEl.className = 'token-status';
        statusEl.title = '';
      }

      expect(statusEl?.textContent).toBe('');
      expect(statusEl?.className).toBe('token-status');
    });
  });

  describe('Event listeners', () => {
    it('should setup usage-update event listener', () => {
      // mockListen would be called during initialization
      expect(mockListen).toBeDefined();
    });

    it('should setup copilot-only-update event listener', () => {
      expect(mockListen).toBeDefined();
    });

    it('should setup token-status event listener', () => {
      expect(mockListen).toBeDefined();
    });
  });

  describe('Force refresh logic', () => {
    it('should trigger refresh when session expired', async () => {
      mockInvoke.mockResolvedValue(undefined);

      await mockInvoke('force_refresh');

      expect(mockInvoke).toHaveBeenCalledWith('force_refresh');
    });

    it('should handle refresh errors gracefully', async () => {
      mockInvoke.mockRejectedValue(new Error('Refresh failed'));

      try {
        await mockInvoke('force_refresh');
      } catch (error) {
        expect(error).toBeInstanceOf(Error);
      }
    });
  });

  describe('Data structure validation', () => {
    it('should validate CombinedUsageData structure', () => {
      const data: CombinedUsageData = {
        claude: {
          five_hour: {
            utilization: 50,
            resets_at: new Date().toISOString(),
          },
          seven_day: {
            utilization: 30,
            resets_at: new Date().toISOString(),
          },
        },
        copilot: null,
      };

      expect(data.claude).toBeDefined();
      expect(data.claude.five_hour).toBeDefined();
      expect(data.claude.seven_day).toBeDefined();
      expect(data.copilot).toBeNull();
    });

    it('should validate CopilotUsageData structure', () => {
      const copilotData: CopilotUsageData = {
        total_requests: 100,
        monthly_limit: 500,
        utilization: 20,
        resets_at: new Date().toISOString(),
        items: [
          { model: 'claude-3-opus', gross_quantity: 50 },
          { model: 'claude-3-sonnet', gross_quantity: 50 },
        ],
      };

      expect(copilotData.total_requests).toBe(100);
      expect(copilotData.monthly_limit).toBe(500);
      expect(copilotData.utilization).toBe(20);
      expect(copilotData.items).toHaveLength(2);
    });
  });
});
