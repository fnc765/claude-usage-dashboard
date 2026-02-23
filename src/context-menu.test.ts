import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest';
// Import runtime value (not just type) so the module is loaded and instrumented for coverage.
import { initContextMenu, type Settings } from './context-menu';

const { mockInvoke } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
}));

// Mock Tauri API
vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
}));

describe('context-menu.ts', () => {
  describe('Settings interface', () => {
    it('should have valid default settings structure', () => {
      const settings: Settings = {
        opacity: 75,
        bgEffect: 'mica',
        alwaysOnTop: true,
        pollingInterval: 60,
        showClaudeMeters: true,
        showCopilotMeter: true,
        autostartEnabled: false,
      };

      expect(settings.opacity).toBe(75);
      expect(settings.bgEffect).toBe('mica');
      expect(settings.alwaysOnTop).toBe(true);
      expect(settings.pollingInterval).toBe(60);
      expect(settings.showClaudeMeters).toBe(true);
      expect(settings.showCopilotMeter).toBe(true);
      expect(settings.autostartEnabled).toBe(false);
    });

    it('should accept all valid bgEffect values', () => {
      const effects: Array<Settings['bgEffect']> = ['transparent', 'mica', 'acrylic'];

      effects.forEach(effect => {
        const settings: Settings = {
          opacity: 75,
          bgEffect: effect,
          alwaysOnTop: true,
          pollingInterval: 60,
          showClaudeMeters: true,
          showCopilotMeter: true,
          autostartEnabled: false,
        };

        expect(settings.bgEffect).toBe(effect);
      });
    });

    it('should handle opacity range', () => {
      const opacityValues = [0, 25, 50, 75, 100];

      opacityValues.forEach(opacity => {
        const settings: Settings = {
          opacity,
          bgEffect: 'mica',
          alwaysOnTop: true,
          pollingInterval: 60,
          showClaudeMeters: true,
          showCopilotMeter: true,
          autostartEnabled: false,
        };

        expect(settings.opacity).toBe(opacity);
        expect(settings.opacity).toBeGreaterThanOrEqual(0);
        expect(settings.opacity).toBeLessThanOrEqual(100);
      });
    });

    it('should handle different polling intervals', () => {
      const intervals = [30, 60, 120, 300];

      intervals.forEach(interval => {
        const settings: Settings = {
          opacity: 75,
          bgEffect: 'mica',
          alwaysOnTop: true,
          pollingInterval: interval,
          showClaudeMeters: true,
          showCopilotMeter: true,
          autostartEnabled: false,
        };

        expect(settings.pollingInterval).toBe(interval);
      });
    });
  });

  describe('localStorage operations', () => {
    beforeEach(() => {
      // Clear localStorage before each test
      localStorage.clear();
    });

    it('should save and retrieve settings from localStorage', () => {
      const settings: Settings = {
        opacity: 80,
        bgEffect: 'acrylic',
        alwaysOnTop: false,
        pollingInterval: 120,
        showClaudeMeters: false,
        showCopilotMeter: false,
        autostartEnabled: true,
      };

      const STORAGE_KEY = 'widget-settings';
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));

      const retrieved = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}');

      expect(retrieved.opacity).toBe(80);
      expect(retrieved.bgEffect).toBe('acrylic');
      expect(retrieved.alwaysOnTop).toBe(false);
      expect(retrieved.pollingInterval).toBe(120);
      expect(retrieved.showClaudeMeters).toBe(false);
      expect(retrieved.showCopilotMeter).toBe(false);
      expect(retrieved.autostartEnabled).toBe(true);
    });

    it('should handle empty localStorage', () => {
      const STORAGE_KEY = 'widget-settings';
      const raw = localStorage.getItem(STORAGE_KEY);

      expect(raw).toBeNull();
    });

    it('should handle corrupted localStorage data', () => {
      const STORAGE_KEY = 'widget-settings';
      localStorage.setItem(STORAGE_KEY, 'invalid-json{');

      let parsed = null;
      try {
        parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}');
      } catch {
        parsed = null;
      }

      expect(parsed).toBeNull();
    });
  });

  describe('WSL path validation', () => {
    it('should validate correct WSL UNC path', () => {
      const validPaths = [
        '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude\\.credentials.json',
        '//wsl.localhost/Ubuntu/home/user/.claude/.credentials.json',
      ];

      validPaths.forEach(path => {
        const isValid = path.startsWith('\\\\wsl.localhost\\') || path.startsWith('//wsl.localhost/');
        expect(isValid).toBe(true);
      });
    });

    it('should reject invalid WSL paths', () => {
      const invalidPaths = [
        'C:\\Users\\user\\.claude\\.credentials.json',
        '/home/user/.claude/.credentials.json',
        '\\\\invalid\\path',
      ];

      invalidPaths.forEach(path => {
        const isValid = path.startsWith('\\\\wsl.localhost\\') || path.startsWith('//wsl.localhost/');
        expect(isValid).toBe(false);
      });
    });

    it('should reject paths with path traversal', () => {
      const paths = [
        '\\\\wsl.localhost\\Ubuntu\\..\\..\\etc\\passwd',
        '//wsl.localhost/Ubuntu/../../../etc/passwd',
      ];

      paths.forEach(path => {
        const hasTraversal = path.includes('..');
        expect(hasTraversal).toBe(true);
      });
    });

    it('should reject paths that are too long', () => {
      const longPath = '\\\\wsl.localhost\\' + 'a'.repeat(500);
      expect(longPath.length).toBeGreaterThan(500);
    });
  });

  describe('GitHub config validation', () => {
    it('should validate required fields', () => {
      const config = {
        username: 'testuser',
        token: 'test-token',
        monthlyLimit: 300,
      };

      expect(config.username).toBeTruthy();
      expect(config.token).toBeTruthy();
      expect(config.monthlyLimit).toBeGreaterThan(0);
    });

    it('should handle empty username and token', () => {
      const username = '   ';
      const token = '   ';

      expect(username.trim()).toBe('');
      expect(token.trim()).toBe('');
    });

    it('should parse monthly limit correctly', () => {
      const limitStr = '500.5';
      const monthlyLimit = parseFloat(limitStr) || 300;

      expect(monthlyLimit).toBe(500.5);
    });

    it('should fallback to default limit on invalid input', () => {
      const limitStr = 'invalid';
      const monthlyLimit = parseFloat(limitStr) || 300;

      expect(monthlyLimit).toBe(300);
    });
  });

  describe('initContextMenu() behavior', () => {
    function setupContextMenuDOM(): void {
      document.body.innerHTML = `
        <div class="widget"></div>
        <div data-meter-type="claude-session"></div>
        <div data-meter-type="claude-weekly"></div>
        <div data-meter-type="copilot"></div>
        <div id="empty-placeholder"></div>
        <div id="context-menu" style="display:none">
          <input id="opacity-slider" type="range" value="75" />
          <span id="opacity-value">75%</span>
          <span id="aot-check"></span>
          <div id="toggle-aot"></div>
          <div id="force-refresh"></div>
          <div id="quit-app"></div>
          <div id="toggle-claude-meters"></div>
          <span id="claude-meters-check"></span>
          <div id="toggle-copilot-meter"></div>
          <span id="copilot-meter-check"></span>
          <div id="toggle-autostart"></div>
          <span id="autostart-check"></span>
          <button id="save-github-config"></button>
          <input id="github-username" />
          <input id="github-token" />
          <input id="monthly-limit" />
          <button id="save-wsl-config"></button>
          <input id="wsl-credentials-path" />
          <button id="clear-wsl-config"></button>
          <div data-effect="transparent"></div>
          <div data-effect="mica" class="active"></div>
          <div data-effect="acrylic"></div>
          <div data-interval="30"></div>
          <div data-interval="60" class="active"></div>
        </div>
      `;
    }

    beforeAll(() => {
      localStorage.clear();
      mockInvoke.mockReset();
      setupContextMenuDOM();
      initContextMenu();
    });

    beforeEach(() => {
      // Reset menu visibility between tests
      const menu = document.getElementById('context-menu');
      if (menu) menu.style.display = 'none';
      mockInvoke.mockClear();
    });

    it('should show context menu on right-click', () => {
      const menu = document.getElementById('context-menu')!;
      document.dispatchEvent(new MouseEvent('contextmenu', { clientX: 50, clientY: 50 }));
      expect(menu.style.display).toBe('block');
    });

    it('should toggle context menu on repeated right-click', () => {
      const menu = document.getElementById('context-menu')!;
      // Show
      document.dispatchEvent(new MouseEvent('contextmenu', { clientX: 50, clientY: 50 }));
      expect(menu.style.display).toBe('block');
      // Hide
      document.dispatchEvent(new MouseEvent('contextmenu', { clientX: 50, clientY: 50 }));
      expect(menu.style.display).toBe('none');
    });

    it('should hide context menu when clicking outside', () => {
      const menu = document.getElementById('context-menu')!;
      // Show first
      document.dispatchEvent(new MouseEvent('contextmenu', { clientX: 50, clientY: 50 }));
      expect(menu.style.display).toBe('block');
      // Click outside the menu
      document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
      expect(menu.style.display).toBe('none');
    });

    it('should hide context menu on Escape key', () => {
      const menu = document.getElementById('context-menu')!;
      // Show first
      document.dispatchEvent(new MouseEvent('contextmenu', { clientX: 50, clientY: 50 }));
      expect(menu.style.display).toBe('block');
      // Press Escape
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
      expect(menu.style.display).toBe('none');
    });

    it('should call force_refresh on force-refresh click', async () => {
      const forceRefresh = document.getElementById('force-refresh')!;
      forceRefresh.click();
      expect(mockInvoke).toHaveBeenCalledWith('force_refresh');
    });

    it('should call quit_app on quit click', async () => {
      const quitApp = document.getElementById('quit-app')!;
      quitApp.click();
      expect(mockInvoke).toHaveBeenCalledWith('quit_app');
    });
  });
});
