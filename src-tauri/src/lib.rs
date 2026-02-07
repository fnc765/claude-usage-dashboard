use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
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
    #[allow(dead_code)]
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageMeter {
    utilization: f64,
    resets_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageData {
    five_hour: UsageMeter,
    seven_day: UsageMeter,
}

struct AppState {
    latest_usage: Option<UsageData>,
}

struct PollingControl {
    interval_tx: watch::Sender<u64>,
    refresh_notify: Notify,
}

fn credentials_path() -> PathBuf {
    let home = dirs::home_dir().expect("Could not find home directory");
    home.join(".claude").join(".credentials.json")
}

fn read_access_token() -> Result<String, String> {
    let path = credentials_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read credentials: {}", e))?;
    let creds: Credentials = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse credentials: {}", e))?;
    Ok(creds.claude_ai_oauth.access_token)
}

async fn fetch_usage(token: &str) -> Result<UsageData, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("API returned status: {}", resp.status()));
    }

    resp.json::<UsageData>()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
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
    control
        .interval_tx
        .send(seconds)
        .map_err(|e| format!("Failed to set interval: {}", e))
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
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
        })))
        .manage(Arc::clone(&polling_control))
        .setup(move |app| {
            let window = app.get_webview_window("main").unwrap();

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
                .icon(app.default_window_icon().unwrap().clone())
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
            let mut interval_rx = interval_rx;

            tauri::async_runtime::spawn(async move {
                async fn do_fetch(app_handle: &tauri::AppHandle) {
                    let token = match read_access_token() {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("Token error: {}", e);
                            return;
                        }
                    };

                    match fetch_usage(&token).await {
                        Ok(data) => {
                            let state = app_handle.state::<Arc<Mutex<AppState>>>();
                            {
                                let mut s = state.lock().await;
                                s.latest_usage = Some(data.clone());
                            }
                            let _ = app_handle.emit("usage-update", &data);
                        }
                        Err(e) => {
                            eprintln!("Usage fetch error: {}", e);
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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            set_background_effect,
            set_always_on_top,
            force_refresh,
            set_polling_interval,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
