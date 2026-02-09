---
applyTo: "src-tauri/**/*.rs"
---

# Rust コーディング規約

## 基本原則

### ✅ Always Do

- **`cargo fmt`を実行** - コミット前に必須
- **`cargo clippy`をクリーン** - 警告ゼロを維持
- **エラーハンドリング**: `?` 演算子と `Result` 型を使用
- **型注釈**: 複雑な型推論の箇所は明示
- **ドキュメントコメント**: 公開APIには `///` を使用
- **テスト**: 新機能には必ずテスト追加

### ❌ Never Do

- `.unwrap()` / `.expect()` の過度な使用
- `unsafe` の不必要な使用
- ハードコードされた設定値
- 無意味なクローン（`.clone()` の乱用）

---

## コーディングスタイル

### エラーハンドリング

```rust
// ✅ 良い例
use anyhow::Result;

pub fn read_config() -> Result<Config> {
    let contents = std::fs::read_to_string("config.toml")?;
    let config: Config = toml::from_str(&contents)?;
    Ok(config)
}

// ❌ 悪い例
pub fn read_config() -> Config {
    let contents = std::fs::read_to_string("config.toml").unwrap();
    toml::from_str(&contents).unwrap()
}
```

### 所有権とボローイング

```rust
// ✅ 良い例: 参照を使用
pub fn process_data(data: &[u8]) -> Result<()> {
    // 処理
}

// ❌ 悪い例: 不必要なクローン
pub fn process_data(data: Vec<u8>) -> Result<()> {
    // 処理後にdataが消費される
}
```

### Option と Result

```rust
// ✅ 良い例: パターンマッチング
match user.email {
    Some(email) => send_notification(&email),
    None => log::warn!("User has no email"),
}

// ✅ 良い例: combinators
user.email.as_ref().map(|email| send_notification(email));

// ❌ 悪い例
if user.email.is_some() {
    send_notification(&user.email.unwrap());
}
```

---

## Tauri 固有の規約

### コマンドの定義

```rust
// ✅ 良い例
#[tauri::command]
async fn get_usage_data(
    app: tauri::AppHandle,
    date_range: DateRange,
) -> Result<UsageData, String> {
    // String エラーは Tauri が自動的にフロントエンドに伝播
    fetch_usage_data(&app, date_range)
        .await
        .map_err(|e| e.to_string())
}

// 使用例
#[tauri::command]
fn simple_command(value: String) -> Result<String, String> {
    Ok(format!("Processed: {}", value))
}
```

### ステート管理

```rust
// ✅ 良い例: Mutex wrapper
use std::sync::Mutex;
use tauri::State;

struct AppState {
    counter: Mutex<u32>,
}

#[tauri::command]
fn increment(state: State<AppState>) -> u32 {
    let mut counter = state.counter.lock().unwrap();
    *counter += 1;
    *counter
}
```

### イベント送信

```rust
// ✅ 良い例
use tauri::Manager;

#[tauri::command]
async fn process_task(app: tauri::AppHandle) -> Result<(), String> {
    // 進捗をフロントエンドに送信
    app.emit_all("task-progress", 50).unwrap();
    
    // 処理...
    
    app.emit_all("task-complete", true).unwrap();
    Ok(())
}
```

---

## ドキュメントコメント

```rust
/// ユーザーの使用状況データを取得します。
///
/// # Arguments
///
/// * `user_id` - 取得対象のユーザーID
/// * `range` - データ取得期間
///
/// # Returns
///
/// `Result<UsageData, Error>` - 成功時は使用状況データ、失敗時はエラー
///
/// # Examples
///
/// ```
/// let data = get_usage_data(123, DateRange::LastWeek)?;
/// ```
pub fn get_usage_data(user_id: u64, range: DateRange) -> Result<UsageData> {
    // 実装
}
```

---

## テスト

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_total() {
        let data = UsageData {
            duration: 3600,
            count: 10,
        };
        assert_eq!(calculate_total(&data), 36000);
    }

    #[tokio::test]
    async fn test_async_function() {
        let result = fetch_data().await;
        assert!(result.is_ok());
    }
}
```

---

## パフォーマンス

```rust
// ✅ 良い例: イテレータチェーン
let sum: i32 = numbers
    .iter()
    .filter(|&&n| n > 0)
    .map(|&n| n * 2)
    .sum();

// ❌ 悪い例: 中間コレクション
let positives: Vec<_> = numbers.iter().filter(|&&n| n > 0).collect();
let doubled: Vec<_> = positives.iter().map(|&n| n * 2).collect();
let sum: i32 = doubled.iter().sum();
```

---

## 並行処理

```rust
// ✅ 良い例: tokio を使用
use tokio::task;

async fn parallel_fetch() -> Result<Vec<Data>> {
    let tasks: Vec<_> = urls
        .iter()
        .map(|url| task::spawn(fetch_url(url.clone())))
        .collect();

    let results = futures::future::join_all(tasks).await;
    // エラーハンドリング...
}
```

---

## 依存関係の管理

```toml
# Cargo.toml - バージョンは明示的に
[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
```

---

## ビルド前チェック

実装完了後、以下を実行:

```bash
# フォーマット
cargo fmt

# Lint
cargo clippy -- -D warnings

# テスト
cargo test

# ビルド確認
cargo build --release
```
