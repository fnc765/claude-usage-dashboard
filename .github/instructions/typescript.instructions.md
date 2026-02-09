---
applyTo: "src/**/*.{ts,js}"
---

# TypeScript/JavaScript コーディング規約

## 基本原則

### ✅ Always Do

- **TypeScript を使用** - 型安全性を重視
- **`npm run lint` をクリーン** - ESLint 警告ゼロ
- **型注釈**: 関数の戻り値と引数に明示的な型
- **非同期処理**: `async/await` を使用（Promise チェーンより優先）
- **エラーハンドリング**: try-catch で適切に処理
- **コメント**: 複雑なロジックには説明を追加

### ❌ Never Do

- `any` 型の乱用
- グローバル変数の使用
- 長大な関数（50行以上）
- コンソールログの本番残留

---

## コーディングスタイル

### 型定義

```typescript
// ✅ 良い例
interface UsageData {
  timestamp: number;
  duration: number;
  userId: string;
}

type DateRange = 'day' | 'week' | 'month' | 'year';

function getUsage(range: DateRange): Promise<UsageData[]> {
  // 実装
}

// ❌ 悪い例
function getUsage(range: any): Promise<any> {
  // any の乱用
}
```

### 非同期処理

```typescript
// ✅ 良い例
async function fetchData(): Promise<UsageData> {
  try {
    const response = await fetch('/api/usage');
    if (!response.ok) {
      throw new Error(`HTTP error: ${response.status}`);
    }
    return await response.json();
  } catch (error) {
    console.error('Failed to fetch data:', error);
    throw error;
  }
}

// ❌ 悪い例
function fetchData(): Promise<any> {
  return fetch('/api/usage')
    .then(res => res.json())
    .catch(err => console.log(err)); // エラーを握りつぶす
}
```

### エラーハンドリング

```typescript
// ✅ 良い例
class ApiError extends Error {
  constructor(public statusCode: number, message: string) {
    super(message);
    this.name = 'ApiError';
  }
}

async function apiCall(): Promise<Data> {
  const response = await fetch('/api/endpoint');
  if (!response.ok) {
    throw new ApiError(response.status, 'Request failed');
  }
  return response.json();
}

// 使用側
try {
  const data = await apiCall();
} catch (error) {
  if (error instanceof ApiError) {
    // API固有のエラー処理
  } else {
    // その他のエラー
  }
}
```

---

## Tauri 固有の規約

### Tauri コマンド呼び出し

```typescript
// ✅ 良い例
import { invoke } from '@tauri-apps/api/tauri';

interface UsageData {
  duration: number;
  count: number;
}

async function getUsageData(userId: string): Promise<UsageData> {
  try {
    return await invoke<UsageData>('get_usage_data', { userId });
  } catch (error) {
    console.error('Tauri command failed:', error);
    throw new Error('Failed to fetch usage data');
  }
}

// ❌ 悪い例
async function getUsageData(userId: any) {
  return invoke('get_usage_data', { userId }); // 型なし、エラーハンドリングなし
}
```

### イベントリスナー

```typescript
// ✅ 良い例
import { listen, UnlistenFn } from '@tauri-apps/api/event';

interface ProgressEvent {
  percent: number;
  message: string;
}

let unlisten: UnlistenFn | null = null;

async function setupListener(): Promise<void> {
  unlisten = await listen<ProgressEvent>('task-progress', (event) => {
    console.log(`Progress: ${event.payload.percent}%`);
    updateUI(event.payload);
  });
}

function cleanup(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
}
```

### ウィンドウ管理

```typescript
// ✅ 良い例
import { appWindow } from '@tauri-apps/api/window';

async function minimizeWindow(): Promise<void> {
  try {
    await appWindow.minimize();
  } catch (error) {
    console.error('Failed to minimize window:', error);
  }
}
```

---

## DOM 操作

```typescript
// ✅ 良い例: 型安全なDOM操作
function updateElement(id: string, text: string): void {
  const element = document.getElementById(id);
  if (!element) {
    console.warn(`Element '${id}' not found`);
    return;
  }
  element.textContent = text;
}

// より良い例: TypeScript の型ガード
function updateInputValue(id: string, value: string): void {
  const element = document.getElementById(id);
  if (element instanceof HTMLInputElement) {
    element.value = value;
  }
}
```

---

## モジュール構造

```typescript
// ✅ 良い例: 名前付きエクスポート
// src/utils/date.ts
export function formatDate(date: Date): string {
  return date.toLocaleDateString();
}

export function parseDate(dateString: string): Date {
  return new Date(dateString);
}

// 使用側
import { formatDate, parseDate } from './utils/date';

// ❌ 悪い例: デフォルトエクスポートの乱用
export default {
  formatDate: (date: Date) => date.toLocaleDateString(),
  parseDate: (dateString: string) => new Date(dateString),
};
```

---

## 状態管理

```typescript
// ✅ 良い例: イミュータブルな状態更新
interface AppState {
  users: User[];
  currentUser: User | null;
}

function addUser(state: AppState, user: User): AppState {
  return {
    ...state,
    users: [...state.users, user],
  };
}

// ❌ 悪い例: ミュータブルな変更
function addUser(state: AppState, user: User): void {
  state.users.push(user); // 元の配列を変更
}
```

---

## パフォーマンス

```typescript
// ✅ 良い例: 非同期処理の並行実行
async function fetchMultiple(ids: string[]): Promise<Data[]> {
  const promises = ids.map(id => fetchData(id));
  return Promise.all(promises);
}

// ❌ 悪い例: 順次実行
async function fetchMultiple(ids: string[]): Promise<Data[]> {
  const results: Data[] = [];
  for (const id of ids) {
    results.push(await fetchData(id)); // 遅い
  }
  return results;
}
```

---

## デバッグとログ

```typescript
// ✅ 良い例: 環境に応じたログ
const isDev = import.meta.env.DEV;

function debugLog(message: string, data?: unknown): void {
  if (isDev) {
    console.log(`[DEBUG] ${message}`, data);
  }
}

// ❌ 悪い例
console.log('Debug info:', sensitiveData); // 本番に残る
```

---

## TSConfig 推奨設定

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  }
}
```

---

## ビルド前チェック

```bash
# Lint
npm run lint

# 型チェック
npm run type-check  # or tsc --noEmit

# ビルド
npm run build
```
