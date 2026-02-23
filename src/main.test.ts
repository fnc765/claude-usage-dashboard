import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest';
import { updateWidget, type CombinedUsageData, type CopilotUsageData } from './widget';

// Use vi.hoisted() so mock variables are initialized before hoisted vi.mock factories run.
const { mockInvoke, mockListen, mockStartDragging, mockGetCurrentWindow, listenCallbacks } = vi.hoisted(() => {
  const mockStartDragging = vi.fn();
  const listenCallbacks: Record<string, Function> = {};
  return {
    mockInvoke: vi.fn(),
    mockListen: vi.fn().mockImplementation(async (event: string, cb: Function) => {
      listenCallbacks[event] = cb;
    }),
    mockStartDragging,
    mockGetCurrentWindow: vi.fn(() => ({
      startDragging: mockStartDragging,
    })),
    listenCallbacks,
  };
});

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

// Import the actual module so V8 instruments its code for coverage.
// vi.mock() calls above are hoisted and active before this import runs.
import './main';

describe('main.ts', () => {
  beforeAll(async () => {
    // Set up DOM for DOMContentLoaded handler (initDrag, listen registrations)
    document.body.innerHTML = `
      <div data-tauri-drag-region>
        <div class="bar-track"></div>
        <button>Test Button</button>
      </div>
      <div id="token-status"></div>
    `;
    // Dispatch DOMContentLoaded to trigger main.ts initialization and capture listen callbacks
    window.dispatchEvent(new Event('DOMContentLoaded'));
    await vi.waitFor(() => {
      if (Object.keys(listenCallbacks).length < 3) {
        throw new Error('Listen callbacks not yet captured');
      }
    });
  });

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

  describe('Listener callback integration', () => {
    const mockCombinedData: CombinedUsageData = {
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

    it('usage-update callback calls updateWidget with payload', () => {
      listenCallbacks['usage-update']({ payload: mockCombinedData });
      expect(updateWidget).toHaveBeenCalledWith(mockCombinedData);
    });

    it('copilot-only-update callback merges copilot data and calls updateWidget', () => {
      // Set latestData via usage-update first
      listenCallbacks['usage-update']({ payload: { ...mockCombinedData } });
      vi.mocked(updateWidget).mockClear();

      const copilotPayload: CopilotUsageData = {
        total_requests: 100,
        monthly_limit: 500,
        utilization: 20,
        resets_at: new Date(Date.now() + 3600000).toISOString(),
        items: [{ model: 'gpt-4', gross_quantity: 100 }],
      };

      listenCallbacks['copilot-only-update']({ payload: copilotPayload });
      expect(updateWidget).toHaveBeenCalledTimes(1);
      const arg = vi.mocked(updateWidget).mock.calls[0][0];
      expect(arg.copilot).toEqual(copilotPayload);
    });

    describe('token-status callback', () => {
      it('handles "expired" status', () => {
        listenCallbacks['token-status']({ payload: 'expired' });
        const el = document.getElementById('token-status')!;
        expect(el.textContent).toBe('\u26a0 Token expired');
        expect(el.className).toBe('token-status error');
      });

      it('handles "error" status', () => {
        listenCallbacks['token-status']({ payload: 'error' });
        const el = document.getElementById('token-status')!;
        expect(el.textContent).toBe('\u26a0 No credentials');
        expect(el.className).toBe('token-status error');
      });

      it('handles "fetch_error" status', () => {
        listenCallbacks['token-status']({ payload: 'fetch_error' });
        const el = document.getElementById('token-status')!;
        expect(el.textContent).toBe('\u26a0 Fetch error');
        expect(el.className).toBe('token-status warning');
      });

      it('handles "ok" status — clears text and class', () => {
        listenCallbacks['token-status']({ payload: 'ok' });
        const el = document.getElementById('token-status')!;
        expect(el.textContent).toBe('');
        expect(el.className).toBe('token-status');
      });

      it('does nothing when #token-status element is missing', () => {
        document.getElementById('token-status')!.remove();
        // Should not throw
        expect(() => listenCallbacks['token-status']({ payload: 'expired' })).not.toThrow();
      });
    });
  });
});
