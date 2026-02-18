/**
 * Application-wide constants and configuration values
 */

// Threshold percentages for usage warnings
export const USAGE_THRESHOLDS = {
  CRITICAL: 80,
  WARNING: 60,
} as const;

// Time window durations in hours
export const TIME_WINDOWS = {
  SESSION: 5,
  WEEKLY: 168, // 7 days * 24 hours
} as const;

// Time conversion constants
export const TIME_CONVERSION = {
  MINUTES_PER_DAY: 1440,
  MINUTES_PER_HOUR: 60,
  MS_PER_MINUTE: 60000,
  MS_PER_HOUR: 3600000,
} as const;

// Polling intervals in milliseconds
export const POLLING_INTERVALS = {
  USAGE_CHECK: 10000, // 10 seconds
  GITHUB_API: 60000, // 60 seconds
  ERROR_RETRY: 10000, // 10 seconds
} as const;

// Default settings values
export const DEFAULT_SETTINGS = {
  OPACITY: 75,
  BG_EFFECT: "mica" as const,
  ALWAYS_ON_TOP: true,
  POLLING_INTERVAL: 60,
  SHOW_CLAUDE_METERS: true,
  SHOW_COPILOT_METER: true,
  AUTOSTART_ENABLED: false,
  MONTHLY_LIMIT: 300,
} as const;

// UI constraints
export const UI_CONSTRAINTS = {
  MENU_PADDING: 4,
  MAX_PATH_LENGTH: 500,
} as const;

// Storage keys
export const STORAGE_KEYS = {
  WIDGET_SETTINGS: "widget-settings",
} as const;

// Token expiration buffer in milliseconds
export const TOKEN_EXPIRATION_BUFFER = 30000; // 30 seconds
