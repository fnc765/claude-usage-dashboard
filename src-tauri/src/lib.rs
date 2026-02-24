use keyring::Entry;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
#[cfg(target_os = "windows")]
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::{watch, Mutex, Notify};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

// Application constants
const TOKEN_EXPIRATION_BUFFER_MS: u64 = 30_000; // 30 seconds
const MAX_WSL_PATH_LENGTH: usize = 500;
const MAX_RESPONSE_PREVIEW_CHARS: usize = 500;

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
    utilization: f64,
    resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtraUsage {
    is_enabled: bool,
    monthly_limit: f64,
    used_credits: f64,
    utilization: f64,
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

// Security: AppConfig stores only non-sensitive data on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    claude: UsageData,
    #[serde(default)]
    copilot: Option<CopilotUsageData>,
}

struct AppState {
    latest_usage: Option<UsageData>,
    http_client: reqwest::Client,
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
        return Ok(AppConfig { github: None, autostart_enabled: false, wsl: None });
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse config: {}", e))
}

fn read_app_config() -> Result<AppConfig, String> {
    let path = config_path()?;
    read_config_from_path(&path)
}

fn write_config_to_path(path: &std::path::Path, config: &AppConfig) -> Result<(), String> {
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(path, content)
        .map_err(|e| format!("Failed to write config: {}", e))
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
        .map_err(|e| format!("Failed to save token to keyring: {}", e))
}

// Security: Read GitHub token from OS keyring
fn read_github_token(username: &str) -> Result<String, String> {
    let entry = Entry::new("usage-dashboard-github", username)
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
    entry
        .get_password()
        .map_err(|e| format!("Failed to read token from keyring: {}", e))
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

// Read full GitHub config (combines storable config + token from keyring)
fn read_github_config() -> Result<Option<GitHubConfig>, String> {
    let config = read_app_config()?;
    if let Some(gh_storable) = config.github {
        match read_github_token(&gh_storable.username) {
            Ok(token) => Ok(Some(GitHubConfig {
                username: gh_storable.username,
                token,
                monthly_limit: gh_storable.monthly_limit,
            })),
            Err(_) => {
                // Token not found in keyring, return None
                Ok(None)
            }
        }
    } else {
        Ok(None)
    }
}

fn calculate_next_month_reset() -> String {
    use chrono::{Datelike, TimeZone, Utc};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let datetime = chrono::DateTime::<Utc>::from_timestamp(now as i64, 0).unwrap();

    let next_month = if datetime.month() == 12 {
        Utc.with_ymd_and_hms(datetime.year() + 1, 1, 1, 0, 0, 0).unwrap()
    } else {
        Utc.with_ymd_and_hms(datetime.year(), datetime.month() + 1, 1, 0, 0, 0).unwrap()
    };

    next_month.to_rfc3339()
}

struct TokenInfo {
    access_token: String,
    expires_at: u64,
}

fn read_token_info() -> Result<TokenInfo, String> {
    // まずWindows版を試す
    match read_token_info_windows() {
        Ok(token) => return Ok(token),
        Err(windows_err) => {
            // Windows版が失敗した場合、WSL版を試す
            if let Ok(config) = read_app_config() {
                if let Some(wsl_config) = config.wsl {
                    match read_token_info_wsl(&wsl_config.credentials_path) {
                        Ok(token) => return Ok(token),
                        Err(wsl_err) => {
                            // セキュリティ: 詳細なエラーはログに出力し、ユーザーには一般的なメッセージを表示
                            eprintln!("Windows credential error: {}", windows_err);
                            eprintln!("WSL credential error: {}", wsl_err);
                            return Err("Failed to read credentials from both Windows and WSL. Please check your configuration.".to_string());
                        }
                    }
                }
            }
            // WSL設定がない場合はWindows版のエラーを返す
            Err(windows_err)
        }
    }
}

fn read_token_info_windows() -> Result<TokenInfo, String> {
    let path = credentials_path()?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read credentials: {}", e))?;
    let creds: Credentials = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse credentials: {}", e))?;
    Ok(TokenInfo {
        access_token: creds.claude_ai_oauth.access_token,
        expires_at: creds.claude_ai_oauth.expires_at,
    })
}

#[cfg(target_os = "windows")]
fn read_token_info_wsl(wsl_path: &str) -> Result<TokenInfo, String> {
    use std::path::Path;

    // セキュリティ検証: WSL UNCパスであることを確認
    if !wsl_path.starts_with("\\\\wsl.localhost\\") && !wsl_path.starts_with("//wsl.localhost/") {
        return Err("WSL path must start with \\\\wsl.localhost\\".to_string());
    }

    // セキュリティ検証: パストラバーサル攻撃を防ぐ
    if wsl_path.contains("..") {
        return Err("Path traversal detected in WSL path".to_string());
    }

    // セキュリティ検証: パスの最大長チェック（DoS対策）
    if wsl_path.len() > MAX_WSL_PATH_LENGTH {
        return Err(format!("WSL path is too long (max {} characters)", MAX_WSL_PATH_LENGTH));
    }

    // パスが .credentials.json で終わっていない場合、自動的に追加
    let path = Path::new(wsl_path);
    let full_path = if path.extension().is_none() || path.file_name() == Some(std::ffi::OsStr::new(".claude")) {
        // ディレクトリパスの場合、.credentials.json を追加
        path.join(".credentials.json")
    } else {
        path.to_path_buf()
    };

    // セキュリティ検証: 最終的なパスが .credentials.json で終わることを確認
    let path_str = full_path.to_string_lossy();
    if !path_str.ends_with(".credentials.json") {
        return Err("WSL credentials path must end with .credentials.json".to_string());
    }

    // UNCパスを読み取る
    let content = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("Failed to read WSL credentials: {}", e))?;

    let creds: Credentials = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse WSL credentials: {}", e))?;

    Ok(TokenInfo {
        access_token: creds.claude_ai_oauth.access_token,
        expires_at: creds.claude_ai_oauth.expires_at,
    })
}

#[cfg(not(target_os = "windows"))]
fn read_token_info_wsl(_wsl_path: &str) -> Result<TokenInfo, String> {
    Err("WSL credentials are only supported on Windows".to_string())
}

fn is_token_expired(expires_at: u64) -> bool {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    now_ms + TOKEN_EXPIRATION_BUFFER_MS >= expires_at
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
    serde_json::from_str::<UsageData>(&body).map_err(|e| {
        format!("Failed to parse response: {}. Body: {}", e, truncated)
    })
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
        return Err(format!("GitHub API status {}: {}", status, body));
    }

    let body = resp.text().await
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
async fn get_usage(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Result<UsageData, String> {
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
            "mica" => apply_mica(&window, Some(true))
                .map_err(|e| format!("Failed to apply mica: {}", e)),
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

#[tauri::command]
fn force_refresh(control: tauri::State<'_, Arc<PollingControl>>) -> Result<(), String> {
    control.refresh_notify.notify_one();
    Ok(())
}

#[tauri::command]
fn set_polling_interval(
    control: tauri::State<'_, Arc<PollingControl>>,
    seconds: u64,
) -> Result<(), String> {
    if seconds < 10 || seconds > 600 {
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
fn save_github_config(
    username: String,
    token: String,
    monthly_limit: f64,
) -> Result<(), String> {
    // Security: Save token to OS keyring, username and monthly_limit to config file
    save_github_token(&username, &token)?;

    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None
    });

    config.github = Some(GitHubConfigStorable {
        username,
        monthly_limit,
    });

    write_app_config(&config)?;
    Ok(())
}

#[tauri::command]
#[cfg(target_os = "windows")]
async fn is_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| format!("Failed to check autostart status: {}", e))
}

#[tauri::command]
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
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

// Windows以外のプラットフォーム向けのフォールバック実装
#[tauri::command]
#[cfg(not(target_os = "windows"))]
async fn is_autostart_enabled(_app: tauri::AppHandle) -> Result<bool, String> {
    Err("Autostart is only supported on Windows".to_string())
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
async fn enable_autostart(_app: tauri::AppHandle) -> Result<(), String> {
    Err("Autostart is only supported on Windows".to_string())
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
async fn disable_autostart(_app: tauri::AppHandle) -> Result<(), String> {
    Err("Autostart is only supported on Windows".to_string())
}

#[tauri::command]
fn get_wsl_config() -> Result<Option<WslConfig>, String> {
    Ok(read_app_config()?.wsl)
}

#[tauri::command]
fn save_wsl_config(credentials_path: String) -> Result<(), String> {
    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });
    config.wsl = Some(WslConfig { credentials_path });
    write_app_config(&config)?;
    Ok(())
}

#[tauri::command]
fn clear_wsl_config() -> Result<(), String> {
    let mut config = read_app_config().unwrap_or(AppConfig {
        github: None,
        autostart_enabled: false,
        wsl: None,
    });
    config.wsl = None;
    write_app_config(&config)?;
    Ok(())
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

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init());

    #[cfg(target_os = "windows")]
    {
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ));
    }

    builder
        .manage(Arc::new(Mutex::new(AppState {
            latest_usage: None,
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
                    let token_info = match read_token_info() {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("Token error: {}", e);
                            let _ = app_handle.emit("token-status", "error");
                            return;
                        }
                    };

                    if is_token_expired(token_info.expires_at) {
                        eprintln!("Access token expired. Run Claude Code to refresh.");
                        let _ = app_handle.emit("token-status", "expired");
                        return;
                    }

                    let client = {
                        let state = app_handle.state::<Arc<Mutex<AppState>>>();
                        let s = state.lock().await;
                        s.http_client.clone()
                    };

                    let claude_result = fetch_usage(&client, &token_info.access_token).await;

                    // GitHub 設定を読み込み (Security: token is read from OS keyring)
                    let github_config = read_github_config().ok().flatten();

                    // GitHub 使用量取得（設定がある場合のみ）
                    let copilot_result = if let Some(gh) = github_config {
                        fetch_copilot_usage(&client, &gh.username, &gh.token, gh.monthly_limit)
                            .await
                            .ok()
                    } else {
                        None
                    };

                    // 結果を結合して送信
                    match claude_result {
                        Ok(claude_data) => {
                            let combined = CombinedUsageData {
                                claude: claude_data.clone(),
                                copilot: copilot_result,
                            };

                            let _ = app_handle.emit("usage-update", &combined);
                            let _ = app_handle.emit("token-status", "ok");

                            let state = app_handle.state::<Arc<Mutex<AppState>>>();
                            let mut s = state.lock().await;
                            s.latest_usage = Some(claude_data);
                        }
                        Err(e) => {
                            eprintln!("Claude API error: {}", e);
                            let _ = app_handle.emit("token-status", "fetch_error");

                            // Claude 失敗時でも Copilot データは送信
                            if let Some(copilot_data) = copilot_result {
                                let _ = app_handle.emit("copilot-only-update", &copilot_data);
                            }
                        }
                    }
                }

                // Immediate first fetch
                do_fetch(&app_handle).await;

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
                if let Ok(cred_path) = credentials_path() {
                    if let Some(parent) = cred_path.parent() {
                        let (tx, rx) = std_mpsc::channel();
                        let mut watcher: RecommendedWatcher =
                            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                                if let Ok(event) = res {
                                    if event.kind.is_modify() || event.kind.is_create() {
                                        let _ = tx.send(());
                                    }
                                }
                            }) {
                                Ok(w) => w,
                                Err(e) => {
                                    eprintln!("Failed to create file watcher: {}", e);
                                    return;
                                }
                            };

                        if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                            eprintln!("Failed to watch credentials dir: {}", e);
                            return;
                        }

                        eprintln!("Watching credentials file: {}", cred_path.display());

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

                        // Explicit cleanup: drop the watcher to release resources
                        drop(watcher);
                        eprintln!("File watcher resources released");
                    }
                }
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
            is_autostart_enabled,
            enable_autostart,
            disable_autostart,
            get_wsl_config,
            save_wsl_config,
            clear_wsl_config,
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
            utilization: 45.5,
            resets_at: Some("2026-03-01T00:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&meter).unwrap();
        assert!(json.contains("45.5"));
        assert!(json.contains("2026-03-01T00:00:00Z"));

        let deserialized: UsageMeter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.utilization, 45.5);
        assert_eq!(deserialized.resets_at, Some("2026-03-01T00:00:00Z".to_string()));
    }

    #[test]
    fn test_extra_usage_serialization() {
        let extra = ExtraUsage {
            is_enabled: true,
            monthly_limit: 1000.0,
            used_credits: 250.0,
            utilization: 25.0,
        };

        let json = serde_json::to_string(&extra).unwrap();
        let deserialized: ExtraUsage = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.is_enabled, true);
        assert_eq!(deserialized.monthly_limit, 1000.0);
        assert_eq!(deserialized.used_credits, 250.0);
        assert_eq!(deserialized.utilization, 25.0);
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
            credentials_path: r"\\wsl.localhost\Ubuntu-24.04\home\user\.claude\.credentials.json".to_string(),
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
        assert_eq!(config.autostart_enabled, false);
        assert!(config.wsl.is_none());
    }

    #[test]
    fn test_copilot_usage_calculation() {
        let items = vec![
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
        assert_eq!(config.autostart_enabled, false);
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
        assert_eq!(loaded.autostart_enabled, true);
        assert!(loaded.wsl.unwrap().credentials_path.contains("wsl.localhost"));
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
        assert_eq!(config.autostart_enabled, false);
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
        assert_eq!(loaded.autostart_enabled, false);
        assert!(loaded.wsl.is_none());
    }
}
