use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
use tokio::sync::{watch, Mutex, Notify};
use tokio::time::Duration;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubConfig {
    username: String,
    token: String,
    #[serde(default = "default_monthly_limit")]
    monthly_limit: f64,
}

fn default_monthly_limit() -> f64 {
    300.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    github: Option<GitHubConfig>,
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

fn read_app_config() -> Result<AppConfig, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig { github: None });
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse config: {}", e))
}

fn write_app_config(config: &AppConfig) -> Result<(), String> {
    let path = config_path()?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write config: {}", e))
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

fn is_token_expired(expires_at: u64) -> bool {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    now_ms + 30_000 >= expires_at
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

    let truncated: String = body.chars().take(500).collect();
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

    let api_response: serde_json::Value = serde_json::from_str(&body)
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

    let utilization = (total_requests / monthly_limit) * 100.0;
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
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn get_github_config() -> Result<Option<GitHubConfig>, String> {
    Ok(read_app_config()?.github)
}

#[tauri::command]
fn save_github_config(
    username: String,
    token: String,
    monthly_limit: f64,
) -> Result<(), String> {
    let mut config = read_app_config().unwrap_or(AppConfig { github: None });
    config.github = Some(GitHubConfig {
        username,
        token,
        monthly_limit,
    });
    write_app_config(&config)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (interval_tx, interval_rx) = watch::channel(60u64);
    let polling_control = Arc::new(PollingControl {
        interval_tx,
        refresh_notify: Notify::new(),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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

                    // GitHub 設定を読み込み
                    let github_config = read_app_config().ok().and_then(|c| c.github);

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

                // Dynamic polling loop
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
                    }
                }
            });

            // Start credentials file watcher
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
                            // Wait for file change, debounce with 1s timeout
                            if rx.recv().is_ok() {
                                // Drain any additional events within 1 second
                                while rx.recv_timeout(std::time::Duration::from_secs(1)).is_ok() {}
                                eprintln!("Credentials file changed, triggering refresh...");
                                watcher_pc.refresh_notify.notify_one();
                            } else {
                                break;
                            }
                        }
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
