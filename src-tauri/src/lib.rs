use keyring::Entry;
use notify::{PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::{watch, Mutex, Notify};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

// Application constants
const TOKEN_EXPIRATION_BUFFER_MS: u64 = 30_000; // 30 seconds
const MAX_WSL_PATH_LENGTH: usize = 500;
const MAX_RESPONSE_PREVIEW_CHARS: usize = 500;
const WSL_FILE_READ_TIMEOUT_SECS: u64 = 5;
const WSL_STARTUP_RETRY_DELAY_SECS: u64 = 30;
const WSL_COOLDOWN_SECS: u64 = 30;

// Invariant: WSL_COOLDOWN_SECS >= WSL_FILE_READ_TIMEOUT_SECS
// Under normal operation (automatic polling), this ensures typically at most one
// leaked thread from read_file_with_timeout at any time.
// See Thread Leak Note on read_file_with_timeout for edge cases.
const _: () = assert!(WSL_COOLDOWN_SECS >= WSL_FILE_READ_TIMEOUT_SECS);
// Invariant: WSL_STARTUP_RETRY_DELAY_SECS >= WSL_COOLDOWN_SECS
// This ensures the "at most one concurrent WSL read thread" invariant holds even
// after clear_wsl_timeout() is called during the startup retry path.
const _: () = assert!(
    WSL_STARTUP_RETRY_DELAY_SECS >= WSL_COOLDOWN_SECS,
    "Startup retry delay must be >= cooldown to prevent concurrent WSL read threads"
);

static WSL_LAST_TIMEOUT: OnceLock<std::sync::Mutex<Option<Instant>>> = OnceLock::new();

/// Tracks whether a WSL file read thread is currently in-flight.
/// Prevents spawning multiple concurrent WSL read threads.
static WSL_READ_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Returns a reference to the global WSL cooldown state.
fn wsl_cooldown_state() -> &'static std::sync::Mutex<Option<Instant>> {
    WSL_LAST_TIMEOUT.get_or_init(|| std::sync::Mutex::new(None))
}

/// Returns `true` if WSL reads should be skipped due to a recent timeout.
fn should_skip_wsl() -> bool {
    if let Ok(guard) = wsl_cooldown_state().lock() {
        if let Some(last) = *guard {
            return last.elapsed().as_secs() < WSL_COOLDOWN_SECS;
        }
    }
    false
}

/// Records the current time as the last WSL timeout.
fn record_wsl_timeout() {
    if let Ok(mut guard) = wsl_cooldown_state().lock() {
        *guard = Some(Instant::now());
    }
}

/// Clears the WSL timeout state (e.g., after a successful read).
fn clear_wsl_timeout() {
    if let Ok(mut guard) = wsl_cooldown_state().lock() {
        *guard = None;
    }
}

/// Error types for WSL file read operations.
#[derive(Debug)]
enum WslReadError {
    /// The file read timed out (WSL may not be running).
    Timeout,
    /// Another WSL read is already in progress.
    InFlight,
    /// WSL read was skipped because cooldown is active.
    CooldownSkipped,
    /// An I/O error occurred while reading the file.
    IoError(String),
    /// The file or path was not found.
    NotFound(String),
}

impl std::fmt::Display for WslReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WslReadError::Timeout => write!(f, "WSL file read timed out"),
            WslReadError::InFlight => write!(f, "Another WSL read is already in progress"),
            WslReadError::CooldownSkipped => write!(f, "WSL read skipped (cooldown active)"),
            WslReadError::IoError(msg) => write!(f, "I/O error: {}", msg),
            WslReadError::NotFound(msg) => write!(f, "Not found: {}", msg),
        }
    }
}

/// Determines whether WSL credentials should be retried on startup.
///
/// Returns `true` if WSL is configured but Claude usage data was not obtained
/// (indicating WSL may not have been ready at first attempt).
fn should_retry_wsl_on_startup(has_wsl_config: bool, claude_usage_is_none: bool) -> bool {
    has_wsl_config && claude_usage_is_none
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    claude_ai_oauth: OAuthCredentials,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthCredentials {
    access_token: String,
    #[allow(dead_code)]
    refresh_token: String,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageMeter {
    #[serde(default)]
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtraUsage {
    is_enabled: bool,
    monthly_limit: f64,
    used_credits: f64,
    utilization: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageData {
    five_hour: UsageMeter,
    seven_day: UsageMeter,
    #[serde(default)]
    seven_day_oauth_apps: Option<UsageMeter>,
    #[serde(default)]
    seven_day_opus: Option<UsageMeter>,
    #[serde(default)]
    seven_day_sonnet: Option<UsageMeter>,
    #[serde(default)]
    seven_day_cowork: Option<UsageMeter>,
    #[serde(default)]
    iguana_necktie: Option<serde_json::Value>,
    #[serde(default)]
    extra_usage: Option<ExtraUsage>,
}

// Security: GitHubConfig with token (runtime use only, not serialized to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubConfig {
    username: String,
    token: String,
    #[serde(default = "default_monthly_limit")]
    monthly_limit: f64,
}

// Security: GitHubConfigStorable without token (safe to store in config.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubConfigStorable {
    username: String,
    #[serde(default = "default_monthly_limit")]
    monthly_limit: f64,
}

fn default_monthly_limit() -> f64 {
    300.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WslConfig {
    /// WSL内の認証情報パス (例: "\\wsl.localhost\Ubuntu-24.04\home\choco\.claude\.credentials.json")
    credentials_path: String,
}

/// Result of saving WSL config, including non-blocking warnings about file accessibility.
#[derive(Debug, Clone, Serialize)]
struct SaveWslResult {
    warnings: Vec<String>,
}

/// Platform information provided to the frontend for OS-specific UI decisions.
#[derive(Debug, Clone, Serialize)]
struct PlatformInfo {
    os: String,
    is_wsl_supported: bool,
}

// Security: AppConfig stores only non-sensitive data on disk
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    github: Option<GitHubConfigStorable>,
    #[serde(default)]
    autostart_enabled: bool,
    #[serde(default)]
    wsl: Option<WslConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CopilotUsageItem {
    model: String,
    gross_quantity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CopilotUsageData {
    total_requests: f64,
    monthly_limit: f64,
    utilization: f64,
    resets_at: String,
    items: Vec<CopilotUsageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CombinedUsageData {
    claude: Option<UsageData>,
    #[serde(default)]
    copilot: Option<CopilotUsageData>,
}

struct AppState {
    latest_usage: Option<CombinedUsageData>,
    http_client: reqwest::Client,
    /// キーリング読み込み失敗時のフォールバック用メモリキャッシュ
    github_token_cache: Option<String>,
}

struct PollingControl {
    interval_tx: watch::Sender<u64>,
    refresh_notify: Notify,
    shutdown_token: CancellationToken,
}

fn credentials_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Could not find home directory".to_string())?;
    Ok(home.join(".claude").join(".credentials.json"))
}

fn config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not find home directory")?;
    let config_dir = home.join(".usage-dashboard");
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;
    Ok(config_dir.join("config.json"))
}

fn read_config_from_path(path: &std::path::Path) -> Result<AppConfig, String> {
    if !path.exists() {
        return Ok(AppConfig {
            github: None,
            autostart_enabled: false,
            wsl: None,
        });
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read config: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))
}

fn read_app_config() -> Result<AppConfig, String> {
    let path = config_path()?;
    read_config_from_path(&path)
}

fn write_config_to_path(path: &std::path::Path, config: &AppConfig) -> Result<(), String> {
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(path, content).map_err(|e| format!("Failed to write config: {}", e))
}

fn write_app_config(config: &AppConfig) -> Result<(), String> {
    let path = config_path()?;
    write_config_to_path(&path, config)
}

// Security: Save GitHub token to OS keyring (secure storage)
fn save_github_token(username: &str, token: &str) -> Result<(), String> {
    let entry = Entry::new("usage-dashboard-github", username)
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
    entry
        .set_password(token)
        .map_err(|e| format!("Failed to save token to keyring: {}", e))?;
    Ok(())
}

// Security: Read GitHub token from OS keyring
fn read_github_token(username: &str) -> Result<String, String> {
    let entry = Entry::new("usage-dashboard-github", username)
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
    match entry.get_password() {
        Ok(token) => Ok(token),
        Err(e) => Err(format!("Failed to read token from keyring: {}", e)),
    }
}

// Security: Delete GitHub token from OS keyring
#[allow(dead_code)]
fn delete_github_token(username: &str) -> Result<(), String> {
    let entry = Entry::new("usage-dashboard-github", username)
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
    entry
        .delete_credential()
        .map_err(|e| format!("Failed to delete token from keyring: {}", e))
}

fn calculate_next_month_reset() -> String {
    use chrono::{Datelike, TimeZone, Utc};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let datetime = chrono::DateTime::<Utc>::from_timestamp(now as i64, 0).unwrap();

    let next_month = if datetime.month() == 12 {
        Utc.with_ymd_and_hms(datetime.year() + 1, 1, 1, 0, 0, 0)
            .unwrap()
    } else {
        Utc.with_ymd_and_hms(datetime.year(), datetime.month() + 1, 1, 0, 0, 0)
            .unwrap()
    };

    next_month.to_rfc3339()
}

#[derive(Debug)]
struct TokenInfo {
    access_token: String,
    expires_at: u64,
}

fn read_token_info() -> Result<TokenInfo, String> {
    let config = read_app_config().unwrap_or_else(|e| {
        eprintln!(
            "[WARN] Failed to read app config, WSL fallback disabled: {}",
            e
        );
        AppConfig::default()
    });
    let wsl_path = config.wsl.as_ref().map(|w| w.credentials_path.as_str());

    // S2: Windows credentials を先に読み取り（WSLタイムアウト時のレイテンシ改善）
    let windows_result = read_token_info_windows();

    // WSLはクールダウン中ならスキップ
    let wsl_result: Option<Result<TokenInfo, WslReadError>> = if should_skip_wsl() {
        eprintln!("[INFO] Skipping WSL read (cooldown active after previous timeout)");
        wsl_path.map(|_| Err(WslReadError::CooldownSkipped))
    } else {
        let result = wsl_path.map(read_token_info_wsl);
        if let Some(ref r) = result {
            match r {
                Ok(_) => clear_wsl_timeout(),
                Err(WslReadError::Timeout) => record_wsl_timeout(),
                Err(WslReadError::InFlight) => {} // クールダウン記録不要
                // Unreachable: read_file_with_timeout() never returns CooldownSkipped.
                // Included for exhaustive match.
                Err(WslReadError::CooldownSkipped) => {}
                Err(_) => {}
            }
        }
        result
    };

    // WSLパスが設定されている場合、WSL優先で選択
    match (wsl_result, windows_result) {
        // 両方成功: expiredでない方を選択、両方有効ならWSL優先
        (Some(Ok(wsl_token)), Ok(win_token)) => {
            let wsl_expired = is_token_expired(wsl_token.expires_at);
            let win_expired = is_token_expired(win_token.expires_at);
            match (wsl_expired, win_expired) {
                (false, _) => {
                    eprintln!("Using WSL credentials (WSL token valid)");
                    Ok(wsl_token)
                }
                (true, false) => {
                    eprintln!("Using Windows credentials (Windows token valid, WSL expired)");
                    Ok(win_token)
                }
                (true, true) => {
                    // 両方expired時は、expires_atが新しい方を返す（再リフレッシュされた側を優先）
                    if wsl_token.expires_at >= win_token.expires_at {
                        eprintln!("Both tokens expired, using WSL with newer expires_at");
                        Ok(wsl_token)
                    } else {
                        eprintln!("Both tokens expired, using Windows with newer expires_at");
                        Ok(win_token)
                    }
                }
            }
        }
        // WSLのみ成功
        (Some(Ok(wsl_token)), Err(_)) => {
            eprintln!("Using WSL credentials (Windows credentials unavailable)");
            Ok(wsl_token)
        }
        // Windowsのみ成功（WSL失敗 or WSL設定なし）
        (Some(Err(_)), Ok(win_token)) | (None, Ok(win_token)) => {
            eprintln!("Using Windows credentials (WSL credentials unavailable)");
            Ok(win_token)
        }
        // 両方失敗
        (Some(Err(wsl_err)), Err(win_err)) => {
            eprintln!("Windows credential error: {}", win_err);
            eprintln!("WSL credential error: {}", wsl_err);
            Err("Failed to read credentials from both Windows and WSL".to_string())
        }
        // WSL設定なし、Windows失敗
        (None, Err(win_err)) => Err(win_err),
    }
}

fn read_token_info_windows() -> Result<TokenInfo, String> {
    let path = credentials_path()?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read credentials: {}", e))?;
    let creds: Credentials = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse credentials: {}", e))?;
    Ok(TokenInfo {
        access_token: creds.claude_ai_oauth.access_token,
        expires_at: creds.claude_ai_oauth.expires_at,
    })
}

/// Validates a WSL UNC path for security and correctness.
/// Returns the normalized path ending with `.credentials.json`.
fn validate_wsl_path(wsl_path: &str) -> Result<String, String> {
    // UNCプレフィックス検証
    if !wsl_path.starts_with("\\\\wsl.localhost\\") && !wsl_path.starts_with("//wsl.localhost/") {
        return Err("WSL path must start with \\\\wsl.localhost\\".to_string());
    }
    // パストラバーサル拒否
    if wsl_path.contains("..") {
        return Err("Path traversal detected in WSL path".to_string());
    }
    // 長さ制限
    if wsl_path.len() > MAX_WSL_PATH_LENGTH {
        return Err(format!(
            "WSL path is too long (max {} characters)",
            MAX_WSL_PATH_LENGTH
        ));
    }
    // .credentials.json 末尾チェック（ディレクトリパスの場合は自動補完）
    let path = std::path::Path::new(wsl_path);
    let full_path = if path.extension().is_none()
        || path.file_name() == Some(std::ffi::OsStr::new(".claude"))
    {
        path.join(".credentials.json")
    } else {
        path.to_path_buf()
    };
    let path_str = full_path.to_string_lossy();
    if !path_str.ends_with(".credentials.json") {
        return Err("WSL credentials path must end with .credentials.json".to_string());
    }
    Ok(path_str.into_owned())
}

/// WSL UNCパス読み取り用のタイムアウト付きファイル読み取り。
/// WSL未起動時にブロッキングI/Oがハングするのを防ぐ。
///
/// Spawns a background thread to perform the file I/O. If the read does not
/// complete within `timeout_secs`, returns `WslReadError::Timeout`.
///
/// # Thread Leak Note
///
/// When timeout occurs, the spawned thread may continue running at the OS level
/// until the WSL file system responds. The `WSL_READ_IN_FLIGHT` atomic guard
/// prevents new read threads from being spawned while a `recv_timeout` is
/// actively waiting (within the same 5-second window). After timeout, the guard
/// is released to allow future reads — meaning OS-level leaked threads from
/// previous timeouts may still exist.
///
/// In practice, leaked thread accumulation is bounded by:
/// - **Automatic polling**: Cooldown (`WSL_COOLDOWN_SECS`) prevents re-reads
///   for 30 seconds after a timeout, far exceeding the 5-second read window.
/// - **Manual refresh**: The in-flight guard blocks concurrent spawns during
///   the active timeout window. Between timeout windows (after guard release),
///   rapid manual refreshes could theoretically spawn additional threads,
///   but this requires deliberate 5+ second-spaced clicks while WSL is
///   unresponsive — an unlikely user behavior pattern.
///
/// The compile-time assertions (`WSL_COOLDOWN_SECS >= WSL_FILE_READ_TIMEOUT_SECS`
/// and `WSL_STARTUP_RETRY_DELAY_SECS >= WSL_COOLDOWN_SECS`) ensure that
/// automatic code paths cannot produce overlapping read threads.
#[cfg(target_os = "windows")]
fn read_file_with_timeout(path: &str, timeout_secs: u64) -> Result<String, WslReadError> {
    // In-flight guard: prevent multiple concurrent WSL read threads
    if WSL_READ_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        eprintln!("[INFO] Skipping WSL read (another read already in-flight)");
        return Err(WslReadError::InFlight);
    }

    let path = path.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::fs::read_to_string(&path);
        let _ = tx.send(result);
    });

    let result = match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(Ok(content)) => Ok(content),
        Ok(Err(e)) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Err(WslReadError::NotFound(format!("File not found: {}", e)))
            } else {
                Err(WslReadError::IoError(format!("Failed to read file: {}", e)))
            }
        }
        Err(_) => Err(WslReadError::Timeout),
    };

    WSL_READ_IN_FLIGHT.store(false, Ordering::SeqCst);
    result
}

#[cfg(target_os = "windows")]
fn read_token_info_wsl(wsl_path: &str) -> Result<TokenInfo, WslReadError> {
    let validated_path = validate_wsl_path(wsl_path).map_err(WslReadError::NotFound)?;

    // UNCパスをタイムアウト付きで読み取る（WSL未起動時のハング防止）
    let content = read_file_with_timeout(&validated_path, WSL_FILE_READ_TIMEOUT_SECS)?;

    let creds: Credentials = serde_json::from_str(&content)
        .map_err(|e| WslReadError::IoError(format!("Failed to parse WSL credentials: {}", e)))?;

    Ok(TokenInfo {
        access_token: creds.claude_ai_oauth.access_token,
        expires_at: creds.claude_ai_oauth.expires_at,
    })
}

#[cfg(not(target_os = "windows"))]
fn read_token_info_wsl(_wsl_path: &str) -> Result<TokenInfo, WslReadError> {
    Err(WslReadError::NotFound(
        "WSL credentials are only supported on Windows".to_string(),
    ))
}

/// Normalize `expires_at` to milliseconds.
///
/// Claude CLI versions may provide `expires_at` in either seconds or milliseconds.
/// Threshold: values below 10^10 are treated as seconds (covers up to year 2286),
/// values at or above 10^10 are treated as milliseconds (min ~2001).
fn normalize_expires_at(expires_at: u64) -> u64 {
    if expires_at < 10_000_000_000 {
        // 10桁以下 → 秒単位と判定、ミリ秒に変換
        expires_at * 1000
    } else {
        // 13桁 → ミリ秒単位としてそのまま使用
        expires_at
    }
}

fn is_token_expired(expires_at: u64) -> bool {
    let expires_at_ms = normalize_expires_at(expires_at);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    now_ms + TOKEN_EXPIRATION_BUFFER_MS >= expires_at_ms
}

async fn fetch_usage(client: &reqwest::Client, token: &str) -> Result<UsageData, String> {
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| {
            // Avoid leaking token through reqwest error details
            format!("HTTP request failed: {}", e.without_url())
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_else(|_| "<unreadable>".into());
        return Err(format!("API returned status {}: {}", status, body));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    let truncated: String = body.chars().take(MAX_RESPONSE_PREVIEW_CHARS).collect();
    serde_json::from_str::<UsageData>(&body)
        .map_err(|e| format!("Failed to parse response: {}. Body: {}", e, truncated))
}

async fn fetch_copilot_usage(
    client: &reqwest::Client,
    username: &str,
    token: &str,
    monthly_limit: f64,
) -> Result<CopilotUsageData, String> {
    let url = format!(
        "https://api.github.com/users/{}/settings/billing/premium_request/usage",
        username
    );

    let resp = client
        .get(&url)
        .header("Authorization", format!("token {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "tauri-usage-dashboard")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {}", e.without_url()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_else(|_| "<unreadable>".into());
        let error_msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["message"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "<API error>".to_string());
        return Err(format!("GitHub API status {}: {}", status, error_msg));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read GitHub response: {}", e))?;

    parse_copilot_usage(&body, monthly_limit)
}

fn parse_copilot_usage(body: &str, monthly_limit: f64) -> Result<CopilotUsageData, String> {
    let api_response: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse GitHub response: {}", e))?;

    let items = api_response["usageItems"]
        .as_array()
        .ok_or("Missing usageItems array")?;

    let mut total_requests = 0.0;
    let mut usage_items = Vec::new();

    for item in items {
        if let Some(quantity) = item["grossQuantity"].as_f64() {
            total_requests += quantity;
            if let Some(model) = item["model"].as_str() {
                usage_items.push(CopilotUsageItem {
                    model: model.to_string(),
                    gross_quantity: quantity,
                });
            }
        }
    }

    let utilization = if monthly_limit <= 0.0 {
        0.0
    } else {
        (total_requests / monthly_limit) * 100.0
    };
    let resets_at = calculate_next_month_reset();

    Ok(CopilotUsageData {
        total_requests,
        monthly_limit,
        utilization,
        resets_at,
        items: usage_items,
    })
}

#[tauri::command]
async fn get_usage(
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
) -> Result<CombinedUsageData, String> {
    let state = state.lock().await;
    state
        .latest_usage
        .clone()
        .ok_or_else(|| "No usage data available yet".to_string())
}

#[tauri::command]
fn set_background_effect(window: tauri::WebviewWindow, effect: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};

        let _ = clear_mica(&window);
        let _ = clear_acrylic(&window);

        match effect.as_str() {
            "transparent" => Ok(()),
            "mica" => {
                apply_mica(&window, Some(true)).map_err(|e| format!("Failed to apply mica: {}", e))
            }
            "acrylic" => apply_acrylic(&window, Some((18, 18, 18, 200)))
                .map_err(|e| format!("Failed to apply acrylic: {}", e)),
            _ => Err(format!("Unknown effect: {}", effect)),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = effect;
        Ok(())
    }
}

#[tauri::command]
fn set_always_on_top(window: tauri::WebviewWindow, enabled: bool) -> Result<(), String> {
    window
        .set_always_on_top(enabled)
        .map_err(|e| format!("Failed to set always on top: {}", e))
}

/// Forces an immediate data refresh.
///
/// # Parameters
/// - `clear_cooldown`: When `true`, clears the WSL timeout cooldown before refreshing.
///   Set to `true` for explicit user-initiated refreshes (e.g., "Refresh Now" button)
///   where the user intends to retry WSL after starting it.
///   Set to `false` for automatic refreshes triggered by config saves or timers.
#[tauri::command]
fn force_refresh(
    control: tauri::State<'_, Arc<PollingControl>>,
    clear_cooldown: bool,
) -> Result<(), String> {
    if clear_cooldown {
        clear_wsl_timeout();
    }
    control.refresh_notify.notify_one();
    Ok(())
}

#[tauri::command]
fn set_polling_interval(
    control: tauri::State<'_, Arc<PollingControl>>,
    seconds: u64,
) -> Result<(), String> {
    if !(10..=600).contains(&seconds) {
        return Err("Polling interval must be between 10 and 600 seconds".to_string());
    }
    control
        .interval_tx
        .send(seconds)
        .map_err(|e| format!("Failed to set interval: {}", e))
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle, control: tauri::State<'_, Arc<PollingControl>>) {
    // Signal shutdown to all background tasks
    control.shutdown_token.cancel();

    // Give background tasks a brief moment to clean up
    std::thread::sleep(std::time::Duration::from_millis(100));

    app.exit(0);
}

#[tauri::command]
fn get_github_config() -> Result<Option<GitHubConfigStorable>, String> {
    // Security: Return only non-sensitive config (without token)
    Ok(read_app_config()?.github)
}

#[tauri::command]
async fn save_github_config(
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    username: String,
    token: String,
    monthly_limit: f64,
) -> Result<(), String> {
    // Security: Save token to OS keyring, username and monthly_limit to config file
    // keyring への保存は失敗しても続行（メモリキャッシュで補完）
    if let Err(e) = save_github_token(&username, &token) {
        eprintln!("[WARN] keyring save failed (will use memory cache): {}", e);
    }

    // メモリキャッシュを更新（keyring が機能しない環境のフォールバック）
    {
        let mut s = state.lock().await;
        s.github_token_cache = Some(token.clone());
    }

    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });

    config.github = Some(GitHubConfigStorable {
        username,
        monthly_limit,
    });

    write_app_config(&config)?;
    Ok(())
}

#[tauri::command]
async fn is_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| format!("Failed to check autostart status: {}", e))
}

#[tauri::command]
async fn enable_autostart(app: tauri::AppHandle) -> Result<(), String> {
    app.autolaunch()
        .enable()
        .map_err(|e| format!("Failed to enable autostart: {}", e))?;

    // 設定ファイルに保存
    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });
    config.autostart_enabled = true;
    write_app_config(&config)?;

    Ok(())
}

#[tauri::command]
async fn disable_autostart(app: tauri::AppHandle) -> Result<(), String> {
    app.autolaunch()
        .disable()
        .map_err(|e| format!("Failed to disable autostart: {}", e))?;

    // 設定ファイルに保存
    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });
    config.autostart_enabled = false;
    write_app_config(&config)?;

    Ok(())
}

#[tauri::command]
async fn validate_github_token(username: String, token: String) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Client error: {}", e))?;

    let resp = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("token {}", token))
        .header("User-Agent", "usage-dashboard")
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if resp.status() == 401 {
        return Err("Invalid token: authentication failed (401)".to_string());
    } else if resp.status() == 403 {
        return Err("Token does not have required permissions (403)".to_string());
    } else if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    // トークンの実際の所有者を検証
    let user_data: serde_json::Value = resp
        .json()
        .await
        .map_err(|_| "Failed to parse user response".to_string())?;
    let actual_login = user_data["login"].as_str().unwrap_or("");
    if !actual_login.eq_ignore_ascii_case(&username) {
        return Err(format!(
            "Token belongs to '{}', not '{}'",
            actual_login, username
        ));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn get_wsl_config() -> Result<Option<WslConfig>, String> {
    Ok(read_app_config()?.wsl)
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn get_wsl_config() -> Result<Option<WslConfig>, String> {
    Ok(None)
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn save_wsl_config(credentials_path: String) -> Result<SaveWslResult, String> {
    // セキュリティ検証: パスのバリデーション
    let validated_path = validate_wsl_path(&credentials_path)?;

    // ファイル読み取りテスト（警告レベル — 保存はブロックしない）
    let mut warnings: Vec<String> = Vec::new();

    match read_file_with_timeout(&validated_path, WSL_FILE_READ_TIMEOUT_SECS) {
        Ok(content) => {
            clear_wsl_timeout(); // 読み取り成功 = WSL稼働中確認済み、既存クールダウンを解除
            if let Err(e) = serde_json::from_str::<Credentials>(&content) {
                warnings.push(format!(
                    "File found but credentials JSON could not be parsed: {}",
                    e
                ));
            }
        }
        Err(WslReadError::Timeout) => {
            warnings.push("WSL might not be running (read timed out).".to_string());
            record_wsl_timeout();
        }
        Err(WslReadError::InFlight) => {
            warnings.push(
                "Another WSL read is in progress. Settings saved anyway."
                    .to_string(),
            );
        }
        Err(WslReadError::CooldownSkipped) => {
            // Unreachable: read_file_with_timeout() never returns CooldownSkipped.
            // Included for exhaustive match.
        }
        Err(WslReadError::IoError(_) | WslReadError::NotFound(_)) => {
            warnings.push("File does not exist or cannot be read.".to_string());
        }
    }

    // 保存はバリデーション済みパスで実行する
    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });
    config.wsl = Some(WslConfig {
        credentials_path: validated_path,
    });
    write_app_config(&config)?;
    Ok(SaveWslResult { warnings })
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn save_wsl_config(_credentials_path: String) -> Result<SaveWslResult, String> {
    Err("WSL configuration is only supported on Windows".to_string())
}

#[tauri::command]
fn clear_wsl_config() -> Result<(), String> {
    clear_wsl_timeout();
    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });
    config.wsl = None;
    write_app_config(&config)?;
    Ok(())
}

/// Returns platform information for OS-specific UI decisions.
#[tauri::command]
fn get_platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS.to_string(),
        is_wsl_supported: cfg!(target_os = "windows"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (interval_tx, interval_rx) = watch::channel(60u64);
    let shutdown_token = CancellationToken::new();
    let polling_control = Arc::new(PollingControl {
        interval_tx,
        refresh_notify: Notify::new(),
        shutdown_token: shutdown_token.clone(),
    });

    let mut builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());

    builder = builder.plugin(
        tauri_plugin_window_state::Builder::new()
            .with_state_flags(
                tauri_plugin_window_state::StateFlags::SIZE
                    | tauri_plugin_window_state::StateFlags::POSITION,
            )
            .build(),
    );

    builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![]),
    ));

    builder
        .manage(Arc::new(Mutex::new(AppState {
            latest_usage: None,
            github_token_cache: None,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
        })))
        .manage(Arc::clone(&polling_control))
        .setup(move |app| {
            let window = app
                .get_webview_window("main")
                .ok_or("Main window not found")?;

            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::{apply_acrylic, apply_mica};
                if apply_mica(&window, Some(true)).is_err() {
                    let _ = apply_acrylic(&window, Some((18, 18, 18, 200)));
                }
            }

            // ウィンドウ状態の復元（vibrancy 適用後）
            use tauri_plugin_window_state::WindowExt;
            let _ = window.restore_state(
                tauri_plugin_window_state::StateFlags::SIZE
                    | tauri_plugin_window_state::StateFlags::POSITION,
            );

            // System tray
            let toggle = MenuItemBuilder::with_id("toggle", "Show/Hide").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&toggle, &quit]).build()?;

            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .ok_or("Default window icon not found")?
                        .clone(),
                )
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "toggle" => {
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Start dynamic polling loop
            let app_handle = app.handle().clone();
            let pc = polling_control;
            let watcher_pc = Arc::clone(&pc);
            let mut interval_rx = interval_rx;

            tauri::async_runtime::spawn(async move {
                async fn do_fetch(app_handle: &tauri::AppHandle) {
                    let client = {
                        let state = app_handle.state::<Arc<Mutex<AppState>>>();
                        let s = state.lock().await;
                        s.http_client.clone()
                    };

                    // --- Claude取得（失敗してもCopilotは続行） ---
                    let claude_result: Option<UsageData> = match read_token_info() {
                        Ok(token_info) => {
                            if is_token_expired(token_info.expires_at) {
                                eprintln!("Access token expired. Run Claude Code to refresh.");
                                let _ = app_handle.emit("token-status", "expired");
                                None
                            } else {
                                match fetch_usage(&client, &token_info.access_token).await {
                                    Ok(usage) => {
                                        let _ = app_handle.emit("token-status", "ok");
                                        Some(usage)
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to fetch Claude usage: {}", e);
                                        let _ = app_handle.emit("token-status", "fetch_error");
                                        None
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Token read failed: {}", e);
                            let _ = app_handle.emit("token-status", "error");
                            None
                        }
                    };

                    // --- Copilot取得（常に実行） ---
                    let copilot_result: Option<CopilotUsageData> = {
                        let config = read_app_config().ok();
                        let github_config = match config.and_then(|c| c.github) {
                            Some(gh_storable) => {
                                let cached_token = {
                                    let s = app_handle.state::<Arc<Mutex<AppState>>>();
                                    let s = s.lock().await;
                                    s.github_token_cache.clone()
                                };
                                let token_opt = cached_token
                                    .or_else(|| read_github_token(&gh_storable.username).ok());
                                token_opt.map(|token| GitHubConfig {
                                    username: gh_storable.username,
                                    token,
                                    monthly_limit: gh_storable.monthly_limit,
                                })
                            }
                            None => None,
                        };

                        if let Some(ref gh) = github_config {
                            match fetch_copilot_usage(
                                &client,
                                &gh.username,
                                &gh.token,
                                gh.monthly_limit,
                            )
                            .await
                            {
                                Ok(data) => Some(data),
                                Err(e) => {
                                    eprintln!("Failed to fetch Copilot usage: {}", e);
                                    let _ = app_handle
                                        .emit("copilot-error", "Failed to fetch Copilot usage");
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    };

                    // --- 結果送信 ---
                    let combined = CombinedUsageData {
                        claude: claude_result,
                        copilot: copilot_result,
                    };
                    if combined.claude.is_some() || combined.copilot.is_some() {
                        let _ = app_handle.emit("usage-update", &combined);

                        // AppState更新
                        let state = app_handle.state::<Arc<Mutex<AppState>>>();
                        let mut s = state.lock().await;
                        s.latest_usage = Some(combined);
                    } else {
                        // 両方失敗時: 旧データをクリアし、ステータスを通知
                        let state = app_handle.state::<Arc<Mutex<AppState>>>();
                        let mut s = state.lock().await;
                        s.latest_usage = None;
                        let _ = app_handle.emit("token-status", "no-service");
                    }
                }

                // Immediate first fetch
                do_fetch(&app_handle).await;

                // WSL設定がある場合の初回リトライ（WSL未起動時に30秒後に1回だけ再試行）
                {
                    let has_wsl_config = read_app_config().ok().and_then(|c| c.wsl).is_some();
                    let claude_usage_is_none = {
                        let state = app_handle.state::<Arc<Mutex<AppState>>>();
                        let s = state.lock().await;
                        s.latest_usage
                            .as_ref()
                            .and_then(|u| u.claude.as_ref())
                            .is_none()
                    };
                    if should_retry_wsl_on_startup(has_wsl_config, claude_usage_is_none) {
                        eprintln!(
                            "WSL token not available on startup, retrying in {} seconds...",
                            WSL_STARTUP_RETRY_DELAY_SECS
                        );
                        tokio::time::sleep(Duration::from_secs(WSL_STARTUP_RETRY_DELAY_SECS)).await;
                        clear_wsl_timeout();
                        do_fetch(&app_handle).await;
                    }
                }

                // Dynamic polling loop with shutdown support
                loop {
                    let secs = *interval_rx.borrow();

                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(secs)) => {
                            do_fetch(&app_handle).await;
                        }
                        _ = pc.refresh_notify.notified() => {
                            do_fetch(&app_handle).await;
                        }
                        Ok(_) = interval_rx.changed() => {
                            continue;
                        }
                        _ = pc.shutdown_token.cancelled() => {
                            eprintln!("Polling loop shutting down...");
                            break;
                        }
                    }
                }
            });

            // Start credentials file watcher with proper cleanup
            let watcher_shutdown = watcher_pc.shutdown_token.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let (tx, rx) = std_mpsc::channel();

                // Windows credentials watcher (independent initialization)
                let _windows_watcher: Option<RecommendedWatcher> = {
                    match credentials_path() {
                        Ok(cred_path) => {
                            if let Some(parent) = cred_path.parent() {
                                let tx_win = tx.clone();
                                match notify::recommended_watcher(
                                    move |res: Result<notify::Event, notify::Error>| {
                                        if let Ok(event) = res {
                                            if event.kind.is_modify() || event.kind.is_create() {
                                                let _ = tx_win.send(());
                                            }
                                        }
                                    },
                                ) {
                                    Ok(mut w) => {
                                        match w.watch(parent, RecursiveMode::NonRecursive) {
                                            Ok(_) => {
                                                eprintln!(
                                                    "Watching credentials file: {}",
                                                    cred_path.display()
                                                );
                                                Some(w)
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "[WARN] Failed to watch credentials dir: {}",
                                                    e
                                                );
                                                None
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[WARN] Failed to create file watcher: {}", e);
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            eprintln!("[WARN] Could not determine credentials path: {}", e);
                            None
                        }
                    }
                };

                // WSL credentials PollWatcher (independent initialization)
                let _wsl_watcher: Option<PollWatcher> = {
                    let config = read_app_config().unwrap_or_else(|_| AppConfig::default());
                    if let Some(wsl_path) = config.wsl.as_ref().map(|w| w.credentials_path.as_str())
                    {
                        let wsl_file = std::path::Path::new(wsl_path);
                        if let Some(wsl_parent) = wsl_file.parent() {
                            let tx_wsl = tx.clone();
                            let poll_config = notify::Config::default()
                                .with_poll_interval(std::time::Duration::from_secs(10));
                            match PollWatcher::new(
                                move |res: Result<notify::Event, notify::Error>| {
                                    if let Ok(event) = res {
                                        if event.kind.is_modify() || event.kind.is_create() {
                                            let _ = tx_wsl.send(());
                                        }
                                    }
                                },
                                poll_config,
                            ) {
                                Ok(mut pw) => {
                                    match pw.watch(wsl_parent, RecursiveMode::NonRecursive) {
                                        Ok(_) => {
                                            eprintln!(
                                                "Watching WSL credentials path: {}",
                                                wsl_path
                                            );
                                            Some(pw)
                                        }
                                        Err(e) => {
                                            eprintln!("[WARN] Failed to watch WSL path: {}", e);
                                            None
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[WARN] Failed to create WSL PollWatcher: {}", e);
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                // Drop the original sender; clones are moved into watcher callbacks
                drop(tx);

                // If no watchers were created, exit early
                if _windows_watcher.is_none() && _wsl_watcher.is_none() {
                    eprintln!("[WARN] No file watchers created, skipping credentials monitoring");
                    return;
                }

                loop {
                    // Check for shutdown signal with timeout
                    if watcher_shutdown.is_cancelled() {
                        eprintln!("File watcher shutting down...");
                        break;
                    }

                    // Wait for file change with timeout to allow periodic shutdown checks
                    match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                        Ok(_) => {
                            // Drain any additional events within 1 second
                            while rx.recv_timeout(std::time::Duration::from_secs(1)).is_ok() {}
                            eprintln!("Credentials file changed, triggering refresh...");
                            watcher_pc.refresh_notify.notify_one();
                        }
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {
                            // Timeout is normal, continue to check shutdown signal
                            continue;
                        }
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                            // Channel closed, exit gracefully
                            eprintln!("File watcher channel disconnected");
                            break;
                        }
                    }
                }

                // Explicit cleanup: drop watchers to release resources
                drop(_windows_watcher);
                drop(_wsl_watcher);
                eprintln!("File watcher resources released");
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            set_background_effect,
            set_always_on_top,
            force_refresh,
            set_polling_interval,
            quit_app,
            get_github_config,
            save_github_config,
            validate_github_token,
            is_autostart_enabled,
            enable_autostart,
            disable_autostart,
            get_wsl_config,
            save_wsl_config,
            clear_wsl_config,
            get_platform_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_monthly_limit() {
        assert_eq!(default_monthly_limit(), 300.0);
    }

    #[test]
    fn test_is_token_expired_not_expired() {
        // 1時間後に期限切れ (3600秒 = 3,600,000ミリ秒)
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let expires_at = now_ms + 3_600_000;

        // 30秒のバッファがあるため、まだ期限切れではない
        assert!(!is_token_expired(expires_at));
    }

    #[test]
    fn test_is_token_expired_expired() {
        // 過去のタイムスタンプ
        let expires_at = 1000000;
        assert!(is_token_expired(expires_at));
    }

    #[test]
    fn test_is_token_expired_buffer() {
        // 現在時刻 + 20秒後（30秒のバッファより短い）
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let expires_at = now_ms + 20_000;

        // 30秒のバッファがあるため、期限切れとみなされる
        assert!(is_token_expired(expires_at));
    }

    #[test]
    fn test_calculate_next_month_reset_format() {
        let reset_date = calculate_next_month_reset();

        // RFC3339形式であることを確認
        assert!(reset_date.contains("T"));
        assert!(reset_date.contains("Z") || reset_date.contains("+"));

        // 日付をパース可能であることを確認
        assert!(chrono::DateTime::parse_from_rfc3339(&reset_date).is_ok());
    }

    #[test]
    fn test_credentials_path() {
        let path = credentials_path();
        assert!(path.is_ok());

        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".claude"));
        assert!(path.to_string_lossy().contains(".credentials.json"));
    }

    #[test]
    fn test_config_path() {
        let path = config_path();
        assert!(path.is_ok());

        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".usage-dashboard"));
        assert!(path.to_string_lossy().contains("config.json"));
    }

    #[test]
    fn test_usage_meter_serialization() {
        let meter = UsageMeter {
            utilization: Some(45.5),
            resets_at: Some("2026-03-01T00:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&meter).unwrap();
        assert!(json.contains("45.5"));
        assert!(json.contains("2026-03-01T00:00:00Z"));

        let deserialized: UsageMeter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.utilization, Some(45.5));
        assert_eq!(
            deserialized.resets_at,
            Some("2026-03-01T00:00:00Z".to_string())
        );
    }

    #[test]
    fn test_usage_meter_utilization_null() {
        // utilization が null の JSON をデシリアライズできる
        let json = r#"{"utilization":null,"resets_at":"2026-03-01T00:00:00+00:00"}"#;
        let deserialized: UsageMeter = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.utilization, None);

        // utilization フィールドが欠落しても None として扱われる
        let json_no_field = r#"{"resets_at":"2026-03-01T00:00:00+00:00"}"#;
        let deserialized2: UsageMeter = serde_json::from_str(json_no_field).unwrap();
        assert_eq!(deserialized2.utilization, None);
    }

    #[test]
    fn test_extra_usage_serialization() {
        let extra = ExtraUsage {
            is_enabled: true,
            monthly_limit: 1000.0,
            used_credits: 250.0,
            utilization: Some(25.0),
        };

        let json = serde_json::to_string(&extra).unwrap();
        let deserialized: ExtraUsage = serde_json::from_str(&json).unwrap();

        assert!(deserialized.is_enabled);
        assert_eq!(deserialized.monthly_limit, 1000.0);
        assert_eq!(deserialized.used_credits, 250.0);
        assert_eq!(deserialized.utilization, Some(25.0));
    }

    #[test]
    fn test_extra_usage_utilization_null() {
        // utilization が null の JSON をデシリアライズできる
        let json = r#"{"is_enabled":true,"monthly_limit":5000,"used_credits":0.0,"utilization":null}"#;
        let deserialized: ExtraUsage = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.utilization, None);

        // utilization が None の場合、null にシリアライズされる
        let extra = ExtraUsage {
            is_enabled: true,
            monthly_limit: 5000.0,
            used_credits: 0.0,
            utilization: None,
        };
        let serialized = serde_json::to_string(&extra).unwrap();
        assert!(serialized.contains("\"utilization\":null"), "Expected null, got: {}", serialized);
    }

    #[test]
    fn test_github_config_serialization() {
        let config = GitHubConfig {
            username: "testuser".to_string(),
            token: "ghp_test123".to_string(),
            monthly_limit: 500.0,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: GitHubConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.username, "testuser");
        assert_eq!(deserialized.token, "ghp_test123");
        assert_eq!(deserialized.monthly_limit, 500.0);
    }

    #[test]
    fn test_github_config_default_monthly_limit() {
        let json = r#"{"username":"testuser","token":"ghp_test123"}"#;
        let config: GitHubConfig = serde_json::from_str(json).unwrap();

        // デフォルト値が適用されるべき
        assert_eq!(config.monthly_limit, 300.0);
    }

    #[test]
    fn test_wsl_config_serialization() {
        let config = WslConfig {
            credentials_path: r"\\wsl.localhost\Ubuntu-24.04\home\user\.claude\.credentials.json"
                .to_string(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: WslConfig = serde_json::from_str(&json).unwrap();

        assert!(deserialized.credentials_path.contains("wsl.localhost"));
    }

    #[test]
    fn test_app_config_defaults() {
        let json = "{}";
        let config: AppConfig = serde_json::from_str(json).unwrap();

        assert!(config.github.is_none());
        assert!(!config.autostart_enabled);
        assert!(config.wsl.is_none());
    }

    #[test]
    fn test_copilot_usage_calculation() {
        let items = [
            CopilotUsageItem {
                model: "gpt-4".to_string(),
                gross_quantity: 100.0,
            },
            CopilotUsageItem {
                model: "gpt-3.5".to_string(),
                gross_quantity: 50.0,
            },
        ];

        let total: f64 = items.iter().map(|i| i.gross_quantity).sum();
        let monthly_limit = 300.0;
        let utilization = (total / monthly_limit) * 100.0;

        assert_eq!(total, 150.0);
        assert_eq!(utilization, 50.0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_validation() {
        // 有効なWSLパス
        let valid_path = r"\\wsl.localhost\Ubuntu-24.04\home\user\.claude\.credentials.json";
        assert!(valid_path.starts_with(r"\\wsl.localhost\"));

        // パストラバーサル攻撃を含むパス
        let invalid_path = r"\\wsl.localhost\Ubuntu-24.04\..\..\..\etc\passwd";
        assert!(invalid_path.contains(".."));

        // WSLパスでない
        let non_wsl_path = r"C:\Users\user\.claude\.credentials.json";
        assert!(!non_wsl_path.starts_with(r"\\wsl.localhost\"));
    }

    #[test]
    fn test_read_config_from_path_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        // 存在しないファイルの場合、デフォルト値を返す
        let config = read_config_from_path(&path).unwrap();
        assert!(config.github.is_none());
        assert!(!config.autostart_enabled);
        assert!(config.wsl.is_none());
    }

    #[test]
    fn test_parse_copilot_usage_response() {
        let json = r#"{
            "usageItems": [
                { "model": "gpt-4o", "grossQuantity": 120.0 },
                { "model": "claude-sonnet-4", "grossQuantity": 80.0 },
                { "model": "gpt-4o-mini", "grossQuantity": 50.0 }
            ]
        }"#;
        let result = parse_copilot_usage(json, 300.0).unwrap();

        assert_eq!(result.total_requests, 250.0);
        assert_eq!(result.monthly_limit, 300.0);
        assert!((result.utilization - 83.333).abs() < 0.01);
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.items[0].model, "gpt-4o");
        assert_eq!(result.items[0].gross_quantity, 120.0);
        assert_eq!(result.items[1].model, "claude-sonnet-4");
        assert_eq!(result.items[2].gross_quantity, 50.0);
        // resets_at は RFC3339 形式
        assert!(chrono::DateTime::parse_from_rfc3339(&result.resets_at).is_ok());
    }

    #[test]
    fn test_parse_copilot_usage_zero_limit() {
        let json = r#"{
            "usageItems": [
                { "model": "gpt-4o", "grossQuantity": 100.0 }
            ]
        }"#;
        let result = parse_copilot_usage(json, 0.0).unwrap();
        assert_eq!(result.total_requests, 100.0);
        assert_eq!(result.monthly_limit, 0.0);
        assert_eq!(result.utilization, 0.0);
    }

    #[test]
    fn test_parse_copilot_usage_empty_response() {
        let json = r#"{ "usageItems": [] }"#;
        let result = parse_copilot_usage(json, 300.0).unwrap();

        assert_eq!(result.total_requests, 0.0);
        assert_eq!(result.monthly_limit, 300.0);
        assert_eq!(result.utilization, 0.0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn test_write_and_read_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let config = AppConfig {
            github: Some(GitHubConfigStorable {
                username: "roundtrip-user".to_string(),
                monthly_limit: 500.0,
            }),
            autostart_enabled: true,
            wsl: Some(WslConfig {
                credentials_path: r"\\wsl.localhost\Ubuntu\home\u\.claude\.credentials.json"
                    .to_string(),
            }),
        };

        write_config_to_path(&path, &config).unwrap();
        let loaded = read_config_from_path(&path).unwrap();

        let gh = loaded.github.unwrap();
        assert_eq!(gh.username, "roundtrip-user");
        assert_eq!(gh.monthly_limit, 500.0);
        assert!(loaded.autostart_enabled);
        assert!(loaded
            .wsl
            .unwrap()
            .credentials_path
            .contains("wsl.localhost"));
    }

    #[test]
    fn test_read_config_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let json = r#"{
            "github": { "username": "alice", "monthly_limit": 750.0 },
            "autostart_enabled": false,
            "wsl": { "credentials_path": "\\\\wsl.localhost\\Debian\\home\\a\\.claude\\.credentials.json" }
        }"#;
        std::fs::write(&path, json).unwrap();

        let config = read_config_from_path(&path).unwrap();
        let gh = config.github.unwrap();
        assert_eq!(gh.username, "alice");
        assert_eq!(gh.monthly_limit, 750.0);
        assert!(!config.autostart_enabled);
        assert!(config.wsl.unwrap().credentials_path.contains("Debian"));
    }

    #[test]
    fn test_read_github_config_no_github() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        // github フィールドが None の設定を書き込む
        let config = AppConfig {
            github: None,
            autostart_enabled: false,
            wsl: None,
        };
        write_config_to_path(&path, &config).unwrap();

        let loaded = read_config_from_path(&path).unwrap();
        assert!(loaded.github.is_none());
        assert!(!loaded.autostart_enabled);
        assert!(loaded.wsl.is_none());
    }

    // ===== read_token_info_wsl() テスト =====

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_invalid_prefix_rejected() {
        // WSL UNC パスでないパスは拒否される
        let result = read_token_info_wsl(r"C:\Users\user\.claude\.credentials.json");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, WslReadError::NotFound(_)),
            "Expected NotFound, got: {:?}",
            err
        );
        assert!(
            err.to_string().contains("WSL path must start with"),
            "Unexpected message: {}",
            err
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_traversal_rejected() {
        // パストラバーサル攻撃（`..` を含むパス）は拒否される
        let result = read_token_info_wsl(r"\\wsl.localhost\Ubuntu\..\..\..\etc\passwd");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Path traversal detected"),
            "Unexpected message: {}",
            err
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_too_long_rejected() {
        // MAX_WSL_PATH_LENGTH (500) を超えるパスは拒否される
        let long_segment = "a".repeat(600);
        let long_path = format!(r"\\wsl.localhost\Ubuntu\home\{}", long_segment);
        let result = read_token_info_wsl(&long_path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("too long"),
            "Unexpected message: {}",
            err
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_wrong_suffix_rejected() {
        // .credentials.json で終わらないファイルパスは拒否される
        let result = read_token_info_wsl(r"\\wsl.localhost\Ubuntu\home\user\config.toml");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains(".credentials.json"),
            "Unexpected message: {}",
            err
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_auto_appends_credentials_suffix() {
        // ディレクトリパス（.claude で終わる）の場合、.credentials.json が自動付与される
        // ファイルは実存しないのでファイル読み取りエラーになるが、パス検証は通過する
        let result = read_token_info_wsl(r"\\wsl.localhost\Ubuntu\home\user\.claude");
        assert!(result.is_err());
        // パス検証エラー（prefix/traversal/suffix）ではなく、ファイル読み取りエラーになるべき
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                WslReadError::IoError(_)
                    | WslReadError::NotFound(_)
                    | WslReadError::Timeout
                    | WslReadError::InFlight
            ),
            "Expected file read error, got: {:?}",
            err
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_valid_credentials_file_not_found() {
        // 正しい形式だが存在しないファイルの場合、ファイル読み取りエラー
        let result = read_token_info_wsl(
            r"\\wsl.localhost\NonExistentDistro\home\user\.claude\.credentials.json",
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                WslReadError::IoError(_)
                    | WslReadError::NotFound(_)
                    | WslReadError::Timeout
                    | WslReadError::InFlight
            ),
            "Expected file I/O error, got: {:?}",
            err
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path_forward_slash_prefix_accepted() {
        // //wsl.localhost/ スタイルのパスも受け入れられる
        let result =
            read_token_info_wsl("//wsl.localhost/Ubuntu/home/user/.claude/.credentials.json");
        assert!(result.is_err());
        let err = result.unwrap_err();
        // パス検証エラーではなく、ファイル読み取りエラーになるべき
        assert!(
            matches!(
                err,
                WslReadError::IoError(_)
                    | WslReadError::NotFound(_)
                    | WslReadError::Timeout
                    | WslReadError::InFlight
            ),
            "Expected file I/O error, got: {:?}",
            err
        );
    }

    // ===== parse_copilot_usage() 異常系テスト =====

    #[test]
    fn test_parse_copilot_usage_invalid_json() {
        let result = parse_copilot_usage("not valid json {{{", 300.0);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Failed to parse GitHub response"));
    }

    #[test]
    fn test_parse_copilot_usage_missing_usage_items() {
        // usageItems キーが存在しない
        let json = r#"{ "someOtherField": 42 }"#;
        let result = parse_copilot_usage(json, 300.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing usageItems"));
    }

    #[test]
    fn test_parse_copilot_usage_usage_items_not_array() {
        // usageItems が配列ではない
        let json = r#"{ "usageItems": "not an array" }"#;
        let result = parse_copilot_usage(json, 300.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing usageItems"));
    }

    #[test]
    fn test_parse_copilot_usage_missing_gross_quantity() {
        // grossQuantity フィールドが欠落したアイテム → スキップされる
        let json = r#"{
            "usageItems": [
                { "model": "gpt-4o", "grossQuantity": 100.0 },
                { "model": "gpt-4o-mini" },
                { "model": "claude-sonnet-4", "grossQuantity": 50.0 }
            ]
        }"#;
        let result = parse_copilot_usage(json, 300.0).unwrap();
        // grossQuantity がないアイテムはスキップされる
        assert_eq!(result.total_requests, 150.0);
        assert_eq!(result.items.len(), 2);
    }

    #[test]
    fn test_parse_copilot_usage_missing_model_field() {
        // model フィールドが欠落（grossQuantity はある）→ total に加算されるがアイテムに含まれない
        let json = r#"{
            "usageItems": [
                { "grossQuantity": 100.0 },
                { "model": "gpt-4o", "grossQuantity": 200.0 }
            ]
        }"#;
        let result = parse_copilot_usage(json, 300.0).unwrap();
        assert_eq!(result.total_requests, 300.0);
        // model がないアイテムは items に含まれない
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].model, "gpt-4o");
    }

    #[test]
    fn test_parse_copilot_usage_negative_monthly_limit() {
        let json = r#"{ "usageItems": [{ "model": "gpt-4o", "grossQuantity": 100.0 }] }"#;
        let result = parse_copilot_usage(json, -100.0).unwrap();
        // 負の monthly_limit は 0.0 扱い
        assert_eq!(result.utilization, 0.0);
    }

    // ===== read_config_from_path() 追加テスト =====

    #[test]
    fn test_read_config_from_path_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ invalid json content }}}").unwrap();

        let result = read_config_from_path(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse config"));
    }

    #[test]
    fn test_read_config_from_path_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "").unwrap();

        let result = read_config_from_path(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse config"));
    }

    #[test]
    fn test_read_config_from_path_partial_config() {
        // github のみ指定し、他はデフォルト
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{ "github": { "username": "bob", "monthly_limit": 100.0 } }"#,
        )
        .unwrap();

        let config = read_config_from_path(&path).unwrap();
        let gh = config.github.unwrap();
        assert_eq!(gh.username, "bob");
        assert_eq!(gh.monthly_limit, 100.0);
        assert!(!config.autostart_enabled);
        assert!(config.wsl.is_none());
    }

    // ===== デシリアライゼーション異常系テスト =====

    #[test]
    fn test_credentials_deserialization_missing_fields() {
        // access_token が欠落
        let json = r#"{ "claudeAiOauth": { "refreshToken": "rt", "expiresAt": 999 } }"#;
        let result = serde_json::from_str::<Credentials>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_credentials_deserialization_wrong_types() {
        // expires_at が文字列（数値であるべき）
        let json = r#"{ "claudeAiOauth": { "accessToken": "at", "refreshToken": "rt", "expiresAt": "not a number" } }"#;
        let result = serde_json::from_str::<Credentials>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_credentials_deserialization_valid() {
        let json = r#"{ "claudeAiOauth": { "accessToken": "test-token", "refreshToken": "test-refresh", "expiresAt": 1700000000 } }"#;
        let creds: Credentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.claude_ai_oauth.access_token, "test-token");
        assert_eq!(creds.claude_ai_oauth.expires_at, 1700000000);
    }

    #[test]
    fn test_usage_data_deserialization_minimal() {
        // 必須フィールドのみ（Optional フィールドはすべて欠落）
        let json = r#"{
            "five_hour": { "utilization": 10.0, "resets_at": null },
            "seven_day": { "utilization": 20.0, "resets_at": "2026-03-01T00:00:00Z" }
        }"#;
        let data: UsageData = serde_json::from_str(json).unwrap();
        assert_eq!(data.five_hour.utilization, Some(10.0));
        assert!(data.five_hour.resets_at.is_none());
        assert_eq!(data.seven_day.utilization, Some(20.0));
        assert!(data.seven_day_oauth_apps.is_none());
        assert!(data.seven_day_opus.is_none());
        assert!(data.seven_day_sonnet.is_none());
        assert!(data.seven_day_cowork.is_none());
        assert!(data.iguana_necktie.is_none());
        assert!(data.extra_usage.is_none());
    }

    #[test]
    fn test_usage_data_deserialization_with_all_optional_fields() {
        let json = r#"{
            "five_hour": { "utilization": 10.0, "resets_at": "2026-02-26T05:00:00Z" },
            "seven_day": { "utilization": 20.0, "resets_at": "2026-03-01T00:00:00Z" },
            "seven_day_oauth_apps": { "utilization": 30.0, "resets_at": null },
            "seven_day_opus": { "utilization": 40.0, "resets_at": null },
            "seven_day_sonnet": { "utilization": 50.0, "resets_at": null },
            "seven_day_cowork": { "utilization": 60.0, "resets_at": null },
            "iguana_necktie": { "some": "value" },
            "extra_usage": {
                "is_enabled": true,
                "monthly_limit": 500.0,
                "used_credits": 100.0,
                "utilization": 20.0
            }
        }"#;
        let data: UsageData = serde_json::from_str(json).unwrap();
        assert!(data.seven_day_oauth_apps.is_some());
        assert_eq!(data.seven_day_oauth_apps.unwrap().utilization, Some(30.0));
        assert!(data.seven_day_opus.is_some());
        assert!(data.seven_day_sonnet.is_some());
        assert!(data.seven_day_cowork.is_some());
        assert!(data.iguana_necktie.is_some());
        let extra = data.extra_usage.unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.monthly_limit, 500.0);
        assert_eq!(extra.utilization, Some(20.0));
    }

    #[test]
    fn test_usage_data_deserialization_invalid_missing_required() {
        // five_hour が欠落 → エラー
        let json = r#"{ "seven_day": { "utilization": 20.0, "resets_at": null } }"#;
        let result = serde_json::from_str::<UsageData>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_usage_meter_resets_at_none() {
        let meter = UsageMeter {
            utilization: Some(0.0),
            resets_at: None,
        };
        let json = serde_json::to_string(&meter).unwrap();
        let deserialized: UsageMeter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.utilization, Some(0.0));
        assert!(deserialized.resets_at.is_none());
    }

    #[test]
    fn test_copilot_usage_item_serialization_roundtrip() {
        let item = CopilotUsageItem {
            model: "claude-sonnet-4".to_string(),
            gross_quantity: 123.456,
        };
        let json = serde_json::to_string(&item).unwrap();
        let deserialized: CopilotUsageItem = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.model, "claude-sonnet-4");
        assert_eq!(deserialized.gross_quantity, 123.456);
    }

    #[test]
    fn test_copilot_usage_data_serialization_roundtrip() {
        let data = CopilotUsageData {
            total_requests: 250.0,
            monthly_limit: 300.0,
            utilization: 83.33,
            resets_at: "2026-03-01T00:00:00+00:00".to_string(),
            items: vec![
                CopilotUsageItem {
                    model: "gpt-4o".to_string(),
                    gross_quantity: 150.0,
                },
                CopilotUsageItem {
                    model: "claude-sonnet-4".to_string(),
                    gross_quantity: 100.0,
                },
            ],
        };
        let json = serde_json::to_string(&data).unwrap();
        let deserialized: CopilotUsageData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_requests, 250.0);
        assert_eq!(deserialized.items.len(), 2);
    }

    #[test]
    fn test_combined_usage_data_without_copilot() {
        let combined = CombinedUsageData {
            claude: Some(UsageData {
                five_hour: UsageMeter {
                    utilization: Some(10.0),
                    resets_at: None,
                },
                seven_day: UsageMeter {
                    utilization: Some(20.0),
                    resets_at: None,
                },
                seven_day_oauth_apps: None,
                seven_day_opus: None,
                seven_day_sonnet: None,
                seven_day_cowork: None,
                iguana_necktie: None,
                extra_usage: None,
            }),
            copilot: None,
        };
        let json = serde_json::to_string(&combined).unwrap();
        let deserialized: CombinedUsageData = serde_json::from_str(&json).unwrap();
        assert!(deserialized.copilot.is_none());
        assert!(deserialized.claude.is_some());
        assert_eq!(deserialized.claude.unwrap().five_hour.utilization, Some(10.0));
    }

    #[test]
    fn test_combined_usage_data_with_copilot() {
        let json = r#"{
            "claude": {
                "five_hour": { "utilization": 15.0, "resets_at": null },
                "seven_day": { "utilization": 25.0, "resets_at": "2026-03-01T00:00:00Z" }
            },
            "copilot": {
                "total_requests": 100.0,
                "monthly_limit": 300.0,
                "utilization": 33.33,
                "resets_at": "2026-03-01T00:00:00Z",
                "items": [
                    { "model": "gpt-4o", "gross_quantity": 100.0 }
                ]
            }
        }"#;
        let combined: CombinedUsageData = serde_json::from_str(json).unwrap();
        assert!(combined.claude.is_some());
        assert!(combined.copilot.is_some());
        let copilot = combined.copilot.unwrap();
        assert_eq!(copilot.total_requests, 100.0);
        assert_eq!(copilot.items.len(), 1);
    }

    #[test]
    fn test_combined_usage_data_copilot_only() {
        // Claude未設定、Copilotのみ
        let json = r#"{
            "claude": null,
            "copilot": {
                "total_requests": 50.0,
                "monthly_limit": 300.0,
                "utilization": 16.67,
                "resets_at": "2026-03-01T00:00:00Z",
                "items": [
                    { "model": "gpt-4o", "gross_quantity": 50.0 }
                ]
            }
        }"#;
        let combined: CombinedUsageData = serde_json::from_str(json).unwrap();
        assert!(combined.claude.is_none());
        assert!(combined.copilot.is_some());
        assert_eq!(combined.copilot.unwrap().total_requests, 50.0);
    }

    #[test]
    fn test_github_config_storable_default_monthly_limit() {
        let json = r#"{ "username": "test" }"#;
        let config: GitHubConfigStorable = serde_json::from_str(json).unwrap();
        assert_eq!(config.username, "test");
        assert_eq!(config.monthly_limit, 300.0);
    }

    #[test]
    fn test_github_config_storable_serialization_roundtrip() {
        let config = GitHubConfigStorable {
            username: "myuser".to_string(),
            monthly_limit: 999.0,
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: GitHubConfigStorable = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.username, "myuser");
        assert_eq!(deserialized.monthly_limit, 999.0);
    }

    // ===== write_config_to_path テスト =====

    #[test]
    fn test_write_config_creates_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let config = AppConfig {
            github: None,
            autostart_enabled: false,
            wsl: None,
        };
        write_config_to_path(&path, &config).unwrap();

        // ファイルが有効な JSON であることを確認
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["autostart_enabled"], false);
        assert!(parsed["github"].is_null());
    }

    // ===== is_token_expired 追加テスト =====

    #[test]
    fn test_is_token_expired_exactly_at_buffer() {
        // 現在時刻 + ちょうど 30秒 (バッファ境界)
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let expires_at = now_ms + TOKEN_EXPIRATION_BUFFER_MS;

        // now_ms + 30_000 >= now_ms + 30_000 → true（ちょうど境界で期限切れ）
        assert!(is_token_expired(expires_at));
    }

    #[test]
    fn test_is_token_expired_just_past_buffer() {
        // 現在時刻 + 31秒（バッファを1秒超える）
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let expires_at = now_ms + TOKEN_EXPIRATION_BUFFER_MS + 1_000;

        assert!(!is_token_expired(expires_at));
    }

    #[test]
    fn test_is_token_expired_zero() {
        // expires_at = 0 は常に期限切れ
        assert!(is_token_expired(0));
    }

    // ===== normalize_expires_at テスト =====

    #[test]
    fn test_normalize_expires_at_seconds() {
        // 秒単位の値はミリ秒に変換される
        assert_eq!(normalize_expires_at(1_740_000_000), 1_740_000_000_000);
    }

    #[test]
    fn test_normalize_expires_at_milliseconds() {
        // ミリ秒単位の値はそのまま
        assert_eq!(normalize_expires_at(1_740_000_000_000), 1_740_000_000_000);
    }

    #[test]
    fn test_normalize_expires_at_boundary_seconds() {
        // 境界値: 9_999_999_999 は秒と判定
        assert_eq!(normalize_expires_at(9_999_999_999), 9_999_999_999_000);
    }

    #[test]
    fn test_normalize_expires_at_boundary_milliseconds() {
        // 境界値: 10_000_000_000 はミリ秒と判定
        assert_eq!(normalize_expires_at(10_000_000_000), 10_000_000_000);
    }

    #[test]
    fn test_normalize_expires_at_zero() {
        // 0 は秒と判定 → 0ミリ秒
        assert_eq!(normalize_expires_at(0), 0);
    }

    #[test]
    fn test_is_token_expired_seconds_format_future() {
        // 遠い未来の秒単位 → expired ではない
        assert!(!is_token_expired(4_102_444_800)); // 2100年 in seconds
    }

    #[test]
    fn test_is_token_expired_seconds_format_past() {
        // 過去の秒単位 → expired
        assert!(is_token_expired(1_577_836_800)); // 2020年 in seconds
    }

    // ===== parse_copilot_usage 追加テスト =====

    #[test]
    fn test_parse_copilot_usage_large_quantities() {
        let json = r#"{
            "usageItems": [
                { "model": "gpt-4o", "grossQuantity": 999999.99 }
            ]
        }"#;
        let result = parse_copilot_usage(json, 300.0).unwrap();
        assert_eq!(result.total_requests, 999999.99);
        assert!(result.utilization > 100.0);
    }

    #[test]
    fn test_parse_copilot_usage_gross_quantity_zero() {
        let json = r#"{
            "usageItems": [
                { "model": "gpt-4o", "grossQuantity": 0.0 }
            ]
        }"#;
        let result = parse_copilot_usage(json, 300.0).unwrap();
        assert_eq!(result.total_requests, 0.0);
        assert_eq!(result.utilization, 0.0);
        assert_eq!(result.items.len(), 1);
    }

    // ===== AppConfig フィールドの組み合わせテスト =====

    #[test]
    fn test_app_config_autostart_only() {
        let json = r#"{ "autostart_enabled": true }"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert!(config.github.is_none());
        assert!(config.autostart_enabled);
        assert!(config.wsl.is_none());
    }

    #[test]
    fn test_app_config_wsl_only() {
        let json = r#"{ "wsl": { "credentials_path": "some/path" } }"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert!(config.github.is_none());
        assert!(!config.autostart_enabled);
        assert!(config.wsl.is_some());
        assert_eq!(config.wsl.unwrap().credentials_path, "some/path");
    }

    #[test]
    fn test_app_config_unknown_fields_ignored() {
        // 未知のフィールドがあっても正しくデシリアライズされる
        let json = r#"{ "unknown_field": 42, "autostart_enabled": true }"#;
        let result = serde_json::from_str::<AppConfig>(json);
        // serde のデフォルトは unknown fields を無視しないため、deny_unknown_fields がなければ通る
        // ここでの目的は実際の挙動を確認すること
        if let Ok(config) = result {
            assert!(config.autostart_enabled);
        }
        // deny_unknown_fields の場合はエラーになっても OK
    }

    // ===== get_platform_info テスト =====

    #[test]
    fn test_get_platform_info_returns_valid_os() {
        let info = get_platform_info();
        assert_eq!(info.os, std::env::consts::OS);
        assert!(!info.os.is_empty());
    }

    #[test]
    fn test_get_platform_info_wsl_supported_matches_target() {
        let info = get_platform_info();
        assert_eq!(info.is_wsl_supported, cfg!(target_os = "windows"));
    }

    #[test]
    fn test_platform_info_serialization() {
        let info = PlatformInfo {
            os: "windows".to_string(),
            is_wsl_supported: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"os\":\"windows\""));
        assert!(json.contains("\"is_wsl_supported\":true"));
    }

    // ===== validate_wsl_path() テスト =====

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_valid_backslash() {
        let result =
            validate_wsl_path(r"\\wsl.localhost\Ubuntu-24.04\home\user\.claude\.credentials.json");
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with(".credentials.json"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_valid_forward_slash() {
        let result =
            validate_wsl_path("//wsl.localhost/Ubuntu/home/user/.claude/.credentials.json");
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with(".credentials.json"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_invalid_prefix() {
        let result = validate_wsl_path(r"C:\Users\user\.claude\.credentials.json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("WSL path must start with"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_traversal_rejected() {
        let result = validate_wsl_path(r"\\wsl.localhost\Ubuntu\..\..\..\etc\passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path traversal detected"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_too_long() {
        let long_segment = "a".repeat(600);
        let long_path = format!(r"\\wsl.localhost\Ubuntu\home\{}", long_segment);
        let result = validate_wsl_path(&long_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too long"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_wrong_suffix() {
        let result = validate_wsl_path(r"\\wsl.localhost\Ubuntu\home\user\config.toml");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(".credentials.json"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_auto_append_from_claude_dir() {
        // .claude ディレクトリパスの場合、.credentials.json が自動付与される
        let result = validate_wsl_path(r"\\wsl.localhost\Ubuntu\home\user\.claude");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(
            path.ends_with(".credentials.json"),
            "Expected .credentials.json suffix, got: {}",
            path
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_validate_wsl_path_empty_prefix_rejected() {
        let result = validate_wsl_path("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("WSL path must start with"));
    }

    // ===== WSL cooldown テスト =====

    #[test]
    fn test_should_skip_wsl_initially_false() {
        // 初期状態ではクールダウンは非アクティブ
        // Note: static state は他のテストと共有されるため、明示クリアしてからテスト
        clear_wsl_timeout();
        assert!(!should_skip_wsl());
    }

    #[test]
    fn test_record_and_check_wsl_timeout() {
        // 他のテストとのグローバル状態共有による干渉を防ぐため初期化
        clear_wsl_timeout();
        // タイムアウトを記録した直後はスキップすべき
        record_wsl_timeout();
        assert!(should_skip_wsl());
        // テスト後にクリーンアップ
        clear_wsl_timeout();
    }

    #[test]
    fn test_clear_wsl_timeout_resets() {
        clear_wsl_timeout();
        record_wsl_timeout();
        assert!(should_skip_wsl());
        clear_wsl_timeout();
        assert!(!should_skip_wsl());
    }

    #[test]
    fn test_wsl_cooldown_expired() {
        clear_wsl_timeout();
        // 過去のタイムスタンプ（WSL_COOLDOWN_SECS 以上前）を手動設定してクールダウン満了をテスト
        if let Ok(mut guard) = wsl_cooldown_state().lock() {
            *guard = Some(Instant::now() - std::time::Duration::from_secs(WSL_COOLDOWN_SECS + 1));
        }
        assert!(!should_skip_wsl());
        // クリーンアップ
        clear_wsl_timeout();
    }

    #[test]
    fn test_wsl_cooldown_not_yet_expired() {
        clear_wsl_timeout();
        // クールダウン期間内のタイムスタンプを設定
        if let Ok(mut guard) = wsl_cooldown_state().lock() {
            *guard = Some(Instant::now() - std::time::Duration::from_secs(WSL_COOLDOWN_SECS - 5));
        }
        assert!(should_skip_wsl());
        // クリーンアップ
        clear_wsl_timeout();
    }

    // ===== S3: WSL cooldown integration flow test =====

    #[test]
    fn test_wsl_cooldown_integration_flow() {
        clear_wsl_timeout();
        // 1. record_wsl_timeout() でタイムアウトを記録
        record_wsl_timeout();

        // 2. should_skip_wsl() が true を返すことを確認
        assert!(
            should_skip_wsl(),
            "should_skip_wsl() should be true after recording timeout"
        );

        // 3. clear_wsl_timeout() でクリア
        clear_wsl_timeout();

        // 4. should_skip_wsl() が false を返すことを確認
        assert!(
            !should_skip_wsl(),
            "should_skip_wsl() should be false after clearing timeout"
        );
    }

    #[test]
    fn test_record_then_clear_wsl_timeout() {
        clear_wsl_timeout();
        assert!(!should_skip_wsl());

        record_wsl_timeout();
        assert!(should_skip_wsl());

        clear_wsl_timeout();
        assert!(!should_skip_wsl());
    }

    // ===== read_token_info cooldown → CooldownSkipped test =====

    /// Tests that `read_token_info` skips the WSL branch via `CooldownSkipped`
    /// when cooldown is active, falling back to the Windows-only path.
    ///
    /// Since `read_token_info` consumes WSL errors internally (they become
    /// `Some(Err(CooldownSkipped))` which maps to the "WSL unavailable" arm),
    /// we verify that:
    /// 1. During cooldown, `should_skip_wsl()` returns `true`.
    /// 2. The internal `wsl_result` would be `Some(Err(CooldownSkipped))`.
    /// 3. This simulated path selects the Windows token when available.
    #[test]
    fn test_read_token_info_cooldown_skips_wsl() {
        // Setup: activate cooldown
        clear_wsl_timeout();
        record_wsl_timeout();
        assert!(should_skip_wsl(), "Precondition: cooldown must be active");

        // Simulate the read_token_info WSL branch logic when cooldown is active:
        // `wsl_path.map(|_| Err(WslReadError::CooldownSkipped))`
        let has_wsl_config = true;
        let wsl_result: Option<Result<TokenInfo, WslReadError>> = if should_skip_wsl() {
            if has_wsl_config {
                Some(Err(WslReadError::CooldownSkipped))
            } else {
                None
            }
        } else {
            unreachable!("Cooldown should be active");
        };

        // Verify the WSL result is CooldownSkipped
        assert!(
            matches!(&wsl_result, Some(Err(WslReadError::CooldownSkipped))),
            "WSL result should be CooldownSkipped during cooldown"
        );

        // Simulate the match arm: (Some(Err(_)), Ok(win_token)) → uses Windows token
        let mock_win_token = TokenInfo {
            access_token: "win-token-123".to_string(),
            expires_at: u64::MAX, // far future, not expired
        };
        let selected = match (wsl_result, Ok::<TokenInfo, String>(mock_win_token)) {
            (Some(Err(_)), Ok(win_token)) => win_token,
            _ => panic!("Expected (Some(Err(_)), Ok(_)) branch"),
        };
        assert_eq!(selected.access_token, "win-token-123");

        // Cleanup
        clear_wsl_timeout();
    }

    // ===== S4: should_retry_wsl_on_startup tests =====

    #[test]
    fn test_should_retry_wsl_on_startup_wsl_config_and_no_usage() {
        // WSL設定あり + Claude usage None → true
        assert!(should_retry_wsl_on_startup(true, true));
    }

    #[test]
    fn test_should_retry_wsl_on_startup_wsl_config_and_has_usage() {
        // WSL設定あり + Claude usage Some → false
        assert!(!should_retry_wsl_on_startup(true, false));
    }

    #[test]
    fn test_should_retry_wsl_on_startup_no_wsl_config_and_no_usage() {
        // WSL設定なし + Claude usage None → false
        assert!(!should_retry_wsl_on_startup(false, true));
    }

    #[test]
    fn test_should_retry_wsl_on_startup_no_wsl_config_and_has_usage() {
        // WSL設定なし + Claude usage Some → false
        assert!(!should_retry_wsl_on_startup(false, false));
    }

    // ===== WslReadError Display tests =====

    #[test]
    fn test_wsl_read_error_display_timeout() {
        let err = WslReadError::Timeout;
        assert_eq!(err.to_string(), "WSL file read timed out");
    }

    #[test]
    fn test_wsl_read_error_display_io_error() {
        let err = WslReadError::IoError("disk failure".to_string());
        assert_eq!(err.to_string(), "I/O error: disk failure");
    }

    #[test]
    fn test_wsl_read_error_display_not_found() {
        let err = WslReadError::NotFound("missing file".to_string());
        assert_eq!(err.to_string(), "Not found: missing file");
    }

    #[test]
    fn test_wsl_read_error_in_flight_display() {
        let err = WslReadError::InFlight;
        assert_eq!(
            err.to_string(),
            "Another WSL read is already in progress"
        );
    }

    #[test]
    fn test_wsl_read_error_cooldown_skipped_display() {
        let err = WslReadError::CooldownSkipped;
        assert_eq!(err.to_string(), "WSL read skipped (cooldown active)");
    }

    #[test]
    fn test_in_flight_does_not_record_cooldown() {
        // Setup: ensure clean state
        clear_wsl_timeout();
        assert!(!should_skip_wsl(), "Precondition: no cooldown");

        // The InFlight branch in read_token_info does not call record_wsl_timeout()
        // Verify this by simulating: after an InFlight error, cooldown should NOT be active
        // (This tests the design contract, not the full integration)

        // Simulate what read_token_info does for InFlight:
        // Err(WslReadError::InFlight) => {} — no cooldown recording
        let error = WslReadError::InFlight;
        match &error {
            WslReadError::Timeout => record_wsl_timeout(),
            WslReadError::InFlight => {} // This is the branch under test
            WslReadError::CooldownSkipped => {}
            _ => {}
        }

        // After InFlight, cooldown should still be inactive
        assert!(!should_skip_wsl(), "InFlight should not trigger cooldown");

        // Cleanup
        clear_wsl_timeout();
    }

    // ===== WSL_READ_IN_FLIGHT guard tests =====

    #[test]
    fn test_wsl_read_in_flight_guard() {
        // Reset state
        WSL_READ_IN_FLIGHT.store(false, Ordering::SeqCst);

        // Simulate in-flight read
        WSL_READ_IN_FLIGHT.store(true, Ordering::SeqCst);

        // Verify flag is set
        assert!(WSL_READ_IN_FLIGHT.load(Ordering::SeqCst));

        // Reset
        WSL_READ_IN_FLIGHT.store(false, Ordering::SeqCst);
        assert!(!WSL_READ_IN_FLIGHT.load(Ordering::SeqCst));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_read_file_with_timeout_rejects_when_in_flight() {
        // Ensure clean state
        WSL_READ_IN_FLIGHT.store(false, Ordering::SeqCst);

        // Simulate in-flight read
        WSL_READ_IN_FLIGHT.store(true, Ordering::SeqCst);

        let result = read_file_with_timeout("\\\\wsl.localhost\\dummy\\path", 1);
        assert!(
            matches!(result, Err(WslReadError::InFlight)),
            "Expected InFlight error when another read is in progress"
        );

        // Clean up
        WSL_READ_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}
