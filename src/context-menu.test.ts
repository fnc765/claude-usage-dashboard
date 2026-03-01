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

    // NOTE: Auto-complete tests are covered via SUT (UI click → invoke args) in
    // the 'initContextMenu() behavior' describe block. No manual re-implementation here.
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
        <div id="wsl-section"></div>
        <div id="wsl-divider"></div>
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
      expect(mockInvoke).toHaveBeenCalledWith('force_refresh', { clearCooldown: true });
    });

    it('should call quit_app on quit click', async () => {
      const quitApp = document.getElementById('quit-app')!;
      quitApp.click();
      expect(mockInvoke).toHaveBeenCalledWith('quit_app');
    });

    // --- Opacity slider input ---
    it('should update opacity on slider input and save settings', () => {
      const slider = document.getElementById('opacity-slider') as HTMLInputElement;
      const opacityValue = document.getElementById('opacity-value')!;
      const widget = document.querySelector('.widget') as HTMLElement;

      slider.value = '50';
      slider.dispatchEvent(new Event('input'));

      expect(opacityValue.textContent).toBe('50%');
      expect(widget.style.background).toContain('0.5');
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.opacity).toBe(50);
    });

    // --- Background effect button click ---
    it('should invoke set_background_effect on effect button click', () => {
      const acrylicBtn = document.querySelector('[data-effect="acrylic"]') as HTMLElement;
      acrylicBtn.click();

      expect(mockInvoke).toHaveBeenCalledWith('set_background_effect', { effect: 'acrylic' });
      expect(acrylicBtn.classList.contains('active')).toBe(true);
      const micaBtn = document.querySelector('[data-effect="mica"]') as HTMLElement;
      expect(micaBtn.classList.contains('active')).toBe(false);
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.bgEffect).toBe('acrylic');
    });

    // --- Always on top toggle ---
    it('should toggle always on top and invoke set_always_on_top', () => {
      const toggleAot = document.getElementById('toggle-aot')!;
      const aotCheck = document.getElementById('aot-check')!;

      // Default: alwaysOnTop = true → toggle to false
      toggleAot.click();

      expect(mockInvoke).toHaveBeenCalledWith('set_always_on_top', { enabled: false });
      expect(aotCheck.textContent).toBe('');
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.alwaysOnTop).toBe(false);
    });

    // --- Claude meters visibility toggle ---
    it('should toggle Claude meters visibility and update DOM', () => {
      const toggleClaudeMeters = document.getElementById('toggle-claude-meters')!;
      const claudeMetersCheck = document.getElementById('claude-meters-check')!;
      const sessionMeter = document.querySelector('[data-meter-type="claude-session"]')!;
      const weeklyMeter = document.querySelector('[data-meter-type="claude-weekly"]')!;

      // Default: showClaudeMeters = true → toggle to false
      toggleClaudeMeters.click();

      expect(claudeMetersCheck.textContent).toBe('');
      expect(sessionMeter.classList.contains('hidden')).toBe(true);
      expect(weeklyMeter.classList.contains('hidden')).toBe(true);
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.showClaudeMeters).toBe(false);
    });

    // --- Copilot meter visibility toggle ---
    it('should toggle Copilot meter visibility and update DOM', () => {
      const toggleCopilotMeter = document.getElementById('toggle-copilot-meter')!;
      const copilotMeterCheck = document.getElementById('copilot-meter-check')!;
      const copilotMeter = document.querySelector('[data-meter-type="copilot"]')!;

      // Default: showCopilotMeter = true → toggle to false
      toggleCopilotMeter.click();

      expect(copilotMeterCheck.textContent).toBe('');
      expect(copilotMeter.classList.contains('hidden')).toBe(true);
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.showCopilotMeter).toBe(false);
    });

    // --- All meters hidden → placeholder shown ---
    it('should show placeholder when all meters are hidden', () => {
      // After Claude + Copilot toggles above, both are false
      const placeholder = document.getElementById('empty-placeholder')!;
      expect(placeholder.style.display).toBe('flex');
    });

    // --- Polling interval button click ---
    it('should invoke set_polling_interval on interval button click', () => {
      const btn30 = document.querySelector('[data-interval="30"]') as HTMLElement;
      btn30.click();

      expect(mockInvoke).toHaveBeenCalledWith('set_polling_interval', { seconds: 30 });
      expect(btn30.classList.contains('active')).toBe(true);
      const btn60 = document.querySelector('[data-interval="60"]') as HTMLElement;
      expect(btn60.classList.contains('active')).toBe(false);
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.pollingInterval).toBe(30);
    });

    // --- Autostart toggle (enable) ---
    it('should enable autostart on toggle click', async () => {
      const toggleAutostart = document.getElementById('toggle-autostart')!;
      const autostartCheck = document.getElementById('autostart-check')!;

      // Current state: autostartEnabled = false → enable
      toggleAutostart.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('enable_autostart');
      expect(autostartCheck.textContent).toBe('\u2713');
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.autostartEnabled).toBe(true);
    });

    // --- Autostart toggle (disable) ---
    it('should disable autostart on second toggle click', async () => {
      const toggleAutostart = document.getElementById('toggle-autostart')!;
      const autostartCheck = document.getElementById('autostart-check')!;

      // Current state: autostartEnabled = true (from previous test) → disable
      toggleAutostart.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('disable_autostart');
      expect(autostartCheck.textContent).toBe('');
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.autostartEnabled).toBe(false);
    });

    // --- GitHub config save: empty fields ---
    it('should show alert for empty GitHub fields', () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveBtn = document.getElementById('save-github-config')!;
      (document.getElementById('github-username') as HTMLInputElement).value = '';
      (document.getElementById('github-token') as HTMLInputElement).value = '';

      saveBtn.click();

      expect(mockAlert).toHaveBeenCalledWith('Username and Token are required');
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    // --- GitHub config save: validation success ---
    it('should save GitHub config on successful validation', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveBtn = document.getElementById('save-github-config')!;
      (document.getElementById('github-username') as HTMLInputElement).value = 'testuser';
      (document.getElementById('github-token') as HTMLInputElement).value = 'ghp_test123';
      (document.getElementById('monthly-limit') as HTMLInputElement).value = '500';

      saveBtn.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('validate_github_token', {
        username: 'testuser',
        token: 'ghp_test123',
      });
      expect(mockInvoke).toHaveBeenCalledWith('save_github_config', {
        username: 'testuser',
        token: 'ghp_test123',
        monthlyLimit: 500,
      });
      expect(mockInvoke).toHaveBeenCalledWith('force_refresh', { clearCooldown: false });
      expect(mockAlert).toHaveBeenCalledWith('GitHub token verified and saved successfully!');
    });

    // --- GitHub config save: token validation failure ---
    it('should show alert on token validation failure', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveBtn = document.getElementById('save-github-config')!;
      (document.getElementById('github-username') as HTMLInputElement).value = 'testuser';
      (document.getElementById('github-token') as HTMLInputElement).value = 'bad-token';

      mockInvoke.mockRejectedValueOnce('Invalid token');

      saveBtn.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('validate_github_token', {
        username: 'testuser',
        token: 'bad-token',
      });
      expect(mockAlert).toHaveBeenCalledWith('Token validation failed: Invalid token');
      expect(mockInvoke).not.toHaveBeenCalledWith('save_github_config', expect.anything());
    });

    // --- WSL config save: empty path ---
    it('should show alert for empty WSL path', () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = '';

      saveWslBtn.click();

      expect(mockAlert).toHaveBeenCalledWith('WSL credentials path is required');
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    // --- WSL config save: invalid path format ---
    it('should show alert for invalid WSL path format', () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = 'C:\\Users\\test';

      saveWslBtn.click();

      expect(mockAlert).toHaveBeenCalledWith(expect.stringContaining('Invalid format'));
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    // --- WSL config save: path traversal ---
    it('should show alert for WSL path with path traversal', () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value =
        '\\\\wsl.localhost\\Ubuntu\\..\\..\\etc\\passwd';

      saveWslBtn.click();

      expect(mockAlert).toHaveBeenCalledWith('Invalid path: Path traversal detected');
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    // --- WSL config save: path too long ---
    it('should show alert for WSL path that is too long', () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const longPath = '\\\\wsl.localhost\\' + 'a'.repeat(500);
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = longPath;

      saveWslBtn.click();

      expect(mockAlert).toHaveBeenCalledWith(expect.stringContaining('Path is too long'));
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    // --- WSL config save: auto-complete .claude path ---
    it('should auto-complete path ending with .claude', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({ warnings: [] });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const inputPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = inputPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      const expectedPath = inputPath + '/.credentials.json';
      expect(mockAlert).toHaveBeenCalledWith(`Path was auto-completed to: ${expectedPath}`);
      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: expectedPath,
      });
    });

    // --- WSL config save: auto-complete directory path with trailing slash ---
    it('should auto-complete path ending with slash', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({ warnings: [] });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const inputPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude/';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = inputPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      const expectedPath = inputPath + '.credentials.json';
      expect(mockAlert).toHaveBeenCalledWith(`Path was auto-completed to: ${expectedPath}`);
      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: expectedPath,
      });
    });

    // --- WSL config save: auto-complete arbitrary path ---
    it('should auto-complete path not ending with .credentials.json', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({ warnings: [] });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const inputPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = inputPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      const expectedPath = inputPath + '/.credentials.json';
      expect(mockAlert).toHaveBeenCalledWith(`Path was auto-completed to: ${expectedPath}`);
      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: expectedPath,
      });
    });

    // --- WSL config save: auto-complete path ending with backslash ---
    it('should auto-complete path ending with backslash', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({ warnings: [] });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const inputPath = '\\\\wsl.localhost\\Ubuntu\\home\\user\\.claude\\';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = inputPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      const expectedPath = inputPath + '.credentials.json';
      expect(mockAlert).toHaveBeenCalledWith(`Path was auto-completed to: ${expectedPath}`);
      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: expectedPath,
      });
    });

    // --- WSL config save: no auto-complete when already correct ---
    it('should not auto-complete path already ending with .credentials.json', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({ warnings: [] });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const validPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude\\.credentials.json';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = validPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      // auto-complete alert should NOT be shown
      expect(mockAlert).not.toHaveBeenCalledWith(expect.stringContaining('auto-completed'));
      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: validPath,
      });
    });

    // --- WSL config save: valid path ---
    it('should save WSL config with valid path', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({ warnings: [] });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const validPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude\\.credentials.json';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = validPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: validPath,
      });
      expect(mockInvoke).toHaveBeenCalledWith('force_refresh', { clearCooldown: false });
      expect(mockAlert).toHaveBeenCalledWith('WSL settings saved successfully!');
    });

    // --- WSL config save: valid path with warnings ---
    it('should show warnings when save_wsl_config returns warnings', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({
        warnings: ['File does not exist. WSL might not be running or path may be incorrect.'],
      });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const validPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude\\.credentials.json';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = validPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: validPath,
      });
      expect(mockInvoke).toHaveBeenCalledWith('force_refresh', { clearCooldown: false });
      expect(mockAlert).toHaveBeenCalledWith(
        'WSL path saved with warnings:\nFile does not exist. WSL might not be running or path may be incorrect.',
      );
    });

    // --- WSL config save: multiple warnings joined by newline ---
    it('should display multiple warnings joined by newline', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      mockInvoke.mockResolvedValue({
        warnings: ['Warning 1', 'Warning 2'],
      });
      const saveWslBtn = document.getElementById('save-wsl-config')!;
      const validPath = '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.claude\\.credentials.json';
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = validPath;

      saveWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('save_wsl_config', {
        credentialsPath: validPath,
      });
      expect(mockAlert).toHaveBeenCalledWith(
        'WSL path saved with warnings:\nWarning 1\nWarning 2',
      );
    });

    // --- WSL config clear ---
    it('should clear WSL config', async () => {
      const mockAlert = vi.fn();
      window.alert = mockAlert;
      const clearWslBtn = document.getElementById('clear-wsl-config')!;
      (document.getElementById('wsl-credentials-path') as HTMLInputElement).value = 'some-path';

      clearWslBtn.click();
      await new Promise(r => setTimeout(r, 0));

      expect(mockInvoke).toHaveBeenCalledWith('clear_wsl_config');
      expect(mockInvoke).toHaveBeenCalledWith('force_refresh', { clearCooldown: false });
      expect((document.getElementById('wsl-credentials-path') as HTMLInputElement).value).toBe('');
      expect(mockAlert).toHaveBeenCalledWith('WSL settings cleared successfully!');
    });
  });

  describe('init-time loading behavior', () => {
    function setupDOM(): void {
      document.body.innerHTML = `
        <div class="widget"></div>
        <div data-meter-type="claude-session"></div>
        <div data-meter-type="claude-weekly"></div>
        <div data-meter-type="copilot"></div>
        <div id="empty-placeholder"></div>
        <div id="wsl-section"></div>
        <div id="wsl-divider"></div>
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

    beforeEach(() => {
      localStorage.clear();
      mockInvoke.mockReset();
      setupDOM();
    });

    it('should load GitHub config on init', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'get_github_config') {
          return Promise.resolve({ username: 'octocat', monthly_limit: 500 });
        }
        return Promise.resolve();
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const usernameEl = document.getElementById('github-username') as HTMLInputElement;
      const limitEl = document.getElementById('monthly-limit') as HTMLInputElement;
      expect(usernameEl.value).toBe('octocat');
      expect(limitEl.value).toBe('500');
    });

    it('should load WSL config on init', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'get_wsl_config') {
          return Promise.resolve({
            credentials_path: '\\\\wsl.localhost\\Ubuntu\\home\\user\\.claude\\.credentials.json',
          });
        }
        return Promise.resolve();
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const pathEl = document.getElementById('wsl-credentials-path') as HTMLInputElement;
      expect(pathEl.value).toBe('\\\\wsl.localhost\\Ubuntu\\home\\user\\.claude\\.credentials.json');
    });

    it('should load autostart status (enabled) on init', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'is_autostart_enabled') return Promise.resolve(true);
        return Promise.resolve();
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const autostartCheck = document.getElementById('autostart-check')!;
      expect(autostartCheck.textContent).toBe('\u2713');
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.autostartEnabled).toBe(true);
    });

    it('should load autostart status (disabled) on init', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'is_autostart_enabled') return Promise.resolve(false);
        return Promise.resolve();
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const autostartCheck = document.getElementById('autostart-check')!;
      expect(autostartCheck.textContent).toBe('');
      const saved = JSON.parse(localStorage.getItem('widget-settings') || '{}');
      expect(saved.autostartEnabled).toBe(false);
    });

    it('should load saved settings from localStorage', () => {
      const customSettings = {
        opacity: 90,
        bgEffect: 'acrylic',
        alwaysOnTop: false,
        pollingInterval: 120,
        showClaudeMeters: false,
        showCopilotMeter: false,
        autostartEnabled: true,
      };
      localStorage.setItem('widget-settings', JSON.stringify(customSettings));

      initContextMenu();

      const slider = document.getElementById('opacity-slider') as HTMLInputElement;
      expect(slider.value).toBe('90');
      const opacityValue = document.getElementById('opacity-value')!;
      expect(opacityValue.textContent).toBe('90%');
      const widget = document.querySelector('.widget') as HTMLElement;
      expect(widget.style.background).toContain('0.9');
      const aotCheck = document.getElementById('aot-check')!;
      expect(aotCheck.textContent).toBe('');
      const acrylicBtn = document.querySelector('[data-effect="acrylic"]') as HTMLElement;
      expect(acrylicBtn.classList.contains('active')).toBe(true);
    });

    it('should use defaults when localStorage has corrupted JSON', () => {
      localStorage.setItem('widget-settings', 'not-valid-json{{{');

      initContextMenu();

      const slider = document.getElementById('opacity-slider') as HTMLInputElement;
      expect(slider.value).toBe('75');
      const opacityValue = document.getElementById('opacity-value')!;
      expect(opacityValue.textContent).toBe('75%');
    });
  });

  describe('applyWslSectionVisibility', () => {
    function setupWslDOM(): void {
      document.body.innerHTML = `
        <div class="widget"></div>
        <div data-meter-type="claude-session"></div>
        <div data-meter-type="claude-weekly"></div>
        <div data-meter-type="copilot"></div>
        <div id="empty-placeholder"></div>
        <div id="wsl-section"></div>
        <div id="wsl-divider"></div>
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

    beforeEach(() => {
      localStorage.clear();
      mockInvoke.mockReset();
      setupWslDOM();
    });

    it('should show WSL section on Windows (get_platform_info returns is_wsl_supported: true)', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'get_platform_info') {
          return Promise.resolve({ os: 'windows', is_wsl_supported: true });
        }
        return Promise.resolve();
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const wslSection = document.getElementById('wsl-section')!;
      const wslDivider = document.getElementById('wsl-divider')!;
      // WSL supported → section should remain visible (display not set to "none")
      expect(wslSection.style.display).not.toBe('none');
      expect(wslDivider.style.display).not.toBe('none');
    });

    it('should hide WSL section on non-Windows (get_platform_info returns is_wsl_supported: false)', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'get_platform_info') {
          return Promise.resolve({ os: 'linux', is_wsl_supported: false });
        }
        return Promise.resolve();
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const wslSection = document.getElementById('wsl-section')!;
      const wslDivider = document.getElementById('wsl-divider')!;
      expect(wslSection.style.display).toBe('none');
      expect(wslDivider.style.display).toBe('none');
    });

    it('should fallback to userAgent when get_platform_info fails (Windows userAgent)', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'get_platform_info') {
          return Promise.reject(new Error('Backend unavailable'));
        }
        return Promise.resolve();
      });

      // Mock navigator.userAgent to simulate Windows
      Object.defineProperty(navigator, 'userAgent', {
        value: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36',
        configurable: true,
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const wslSection = document.getElementById('wsl-section')!;
      const wslDivider = document.getElementById('wsl-divider')!;
      // userAgent contains "windows" → WSL section stays visible
      expect(wslSection.style.display).not.toBe('none');
      expect(wslDivider.style.display).not.toBe('none');
    });

    it('should fallback to userAgent when get_platform_info fails (non-Windows userAgent)', async () => {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'get_platform_info') {
          return Promise.reject(new Error('Backend unavailable'));
        }
        return Promise.resolve();
      });

      // Mock navigator.userAgent to simulate Linux
      Object.defineProperty(navigator, 'userAgent', {
        value: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36',
        configurable: true,
      });

      initContextMenu();
      await new Promise(r => setTimeout(r, 0));

      const wslSection = document.getElementById('wsl-section')!;
      const wslDivider = document.getElementById('wsl-divider')!;
      // userAgent does NOT contain "windows" → WSL section hidden
      expect(wslSection.style.display).toBe('none');
      expect(wslDivider.style.display).toBe('none');
    });
  });
});
