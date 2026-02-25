# Claude Code Usage Dashboard

Claude Code と GitHub Copilot の API 使用量をリアルタイムで監視する統合デスクトップウィジェットです。

![Tauri](https://img.shields.io/badge/Tauri-2-blue)
![Rust](https://img.shields.io/badge/Rust-2021-orange)
![License](https://img.shields.io/badge/License-MIT-green)

![Usage Dashboard Screenshot](./image.png)

## 機能

### Claude Code 使用量監視
- **セッション使用量** (5時間枠) と **週間使用量** (7日枠) をプログレスバーで表示
- 使用量が時間経過に対して超過している場合、オレンジ色で警告表示
- 60%/80% の閾値で色が変化 (黄色→赤)
- リセットまでの残り時間をカウントダウン表示
- リセット時刻の到達を検知して自動リフレッシュ

### GitHub Copilot 使用量監視（オプション）
- **月間使用量**をプログレスバーで表示
- GitHub Personal Access Token による認証
- 緑系の色スキームで Claude とは視覚的に区別
- Context Menu から GitHub PAT を設定可能

### カスタマイズ・操作
- **メーター表示切替**: Claude メーター・Copilot メーター を個別に表示/非表示可能
  - 設定は localStorage に永続化され、アプリ再起動後も保持される
- **自動起動**: ログイン時に自動的にアプリを起動
  - Context Menu から有効/無効を切り替え可能
- 常に最前面に表示 (トグル可)
- ウィンドウの透過度・背景エフェクト (Mica / Acrylic) のカスタマイズ
- ポーリング間隔の変更 (30秒〜5分)
- システムトレイからの表示/非表示切り替え
- ドラッグ移動・リサイズ対応

## 対応プラットフォーム

- **Windows** (Windows 10/11)
- **Linux** (Ubuntu, Debian, Fedora など)
- **macOS** (keyring は `apple-native` 実装済み・動作未検証)

### WSL での使用について

- **WSL内のClaude Code**: WSL環境でClaude Codeを使用している場合、WSLの認証ファイルを右クリックメニューから指定してください
- **認証ファイルのパス形式**: UNC 形式で指定する必要があります
  ```
  \\wsl.localhost\<ディストリビューション名>\home\<ユーザー名>\.claude\.credentials.json
  ```
  例: `\\wsl.localhost\Ubuntu-24.04\home\user\.claude\.credentials.json`
- 入力後、コンテキストメニュー内の **Save** ボタンをクリックして保存してください

## 前提条件

### 必須
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) がインストール済みで、OAuth 認証が完了していること
  - `~/.claude/.credentials.json` に認証情報が保存されている必要があります
- [Node.js](https://nodejs.org/) (v18 以上)
- [pnpm](https://pnpm.io/)
- [Rust](https://www.rust-lang.org/tools/install)

### オプション (GitHub Copilot 監視を利用する場合)
- GitHub Personal Access Token (PAT) が必要です
  - スコープ: `copilot` 権限が必要
  - [GitHub Settings](https://github.com/settings/tokens) から生成できます

## インストール方法

### 方法1: ビルド済みパッケージを使用（推奨）

#### Linux (Ubuntu/Debian)

```bash
# 1. GitHubのReleaseページから.debファイルをダウンロード
#    https://github.com/のリリースページから最新の .deb ファイルを取得してください
wget <release-url>/usage-dashboard_0.1.0_amd64.deb

# 2. インストール
sudo dpkg -i usage-dashboard_0.1.0_amd64.deb

# 依存関係のエラーが出た場合
sudo apt-get install -f

# 3. 実行
usage-dashboard
```

#### Linux (AppImage - 全ディストリビューション対応)

```bash
# 1. AppImageをダウンロード
#    GitHubのReleaseページから最新の .AppImage ファイルを取得してください
wget <release-url>/usage-dashboard_0.1.0_amd64.AppImage

# 2. 実行権限を付与
chmod +x usage-dashboard_0.1.0_amd64.AppImage

# 3. 実行
./usage-dashboard_0.1.0_amd64.AppImage
```

#### Windows

GitHub のリリースページから `.msi` インストーラーをダウンロードして実行してください。

### 方法2: ソースからビルド

```bash
# 依存パッケージのインストール
pnpm install

# 開発モードで起動
pnpm tauri dev

# プロダクションビルド
pnpm tauri build
```

## 使い方

### 基本操作

1. アプリを起動すると、小さなウィジェットが画面上に表示されます
2. ウィジェットをドラッグして好きな位置に配置できます
3. **右クリック** でコンテキストメニューを開き、以下をカスタマイズできます:
   - 透過度 (Opacity)
   - 背景エフェクト (Transparent / Mica / Acrylic) ※Windows専用
   - 常に最前面表示の ON/OFF
   - ポーリング間隔
   - 手動リフレッシュ
   - **メーター表示切替**: Claude メーター / GitHub Copilot メーター を個別に表示/非表示
   - **自動起動設定**: ログイン時の自動起動を ON/OFF
4. システムトレイアイコンからウィジェットの表示/非表示を切り替えられます

### GitHub Copilot の設定 (オプション)

GitHub Copilot の使用量を監視したい場合:

1. [GitHub Settings](https://github.com/settings/tokens) で Personal Access Token (PAT) を生成
   - スコープ: `copilot` 権限を付与
2. ウィジェットを **右クリック** してコンテキストメニューを開く
3. メニュー内の GitHub Copilot セクションで以下を入力する:
   - **Username**: GitHub ユーザー名
   - **Token**: 生成した PAT
   - **Monthly Limit**: 月間の上限使用量（デフォルト: 300）
4. **Save** ボタンをクリックして保存

> トークンは OS キーリング（Windows: Windows Credential Manager、macOS: Keychain、Linux: Secret Service）に安全に保存されます。`config.json` には書き込まれません。

## データの保存場所

| データ種別 | 保存先 |
|-----------|-------|
| アプリ設定 (WSL パス、GitHub ユーザー名など) | `~/.usage-dashboard/config.json` |
| GitHub PAT (トークン) | OS キーリング (Windows Credential Manager / macOS Keychain / Linux Secret Service) |
| UI 設定 (透過度、表示切替など) | `localStorage` (アプリ内) |

## テスト

```bash
# フロントエンド (TypeScript)
pnpm test                       # 単体テスト
pnpm test:coverage              # カバレッジ付き

# バックエンド (Rust)
pnpm test:coverage:rust:text    # カバレッジ (テキスト)
pnpm test:coverage:rust         # カバレッジ (HTML出力 → coverage-rust/)

# 両方まとめて実行
pnpm test:coverage:all
```

## 技術スタック

- **フロントエンド**: TypeScript + HTML/CSS (Vanilla)
- **バックエンド**: Rust (Tauri 2)
- **ビルドツール**: Vite
- **パッケージマネージャー**: pnpm

## ライセンス

[MIT](LICENSE)
