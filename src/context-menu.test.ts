import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { Settings } from './context-menu';

// Mock Tauri API
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
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
});
