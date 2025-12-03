# version-lsp 設計ドキュメント

生成日: 2025-12-03
ジェネレーター: requirements-analysis

## システム概要

### 目的
複数の言語・ツールのパッケージ管理ファイル（package.json, Cargo.toml, go.mod, GitHub Actions workflow等）に記載されているパッケージバージョンをチェックし、最新バージョンが利用可能な場合にLanguage Server Protocol (LSP)を通じてエディタ上でリアルタイムに表示するLSPサーバー。

### 解決する問題
- パッケージの依存関係が古くなっていることに気づきにくい
- 各言語・ツールごとに異なるバージョンチェック方法を統一的に扱いたい
- 複数のパッケージファイルを手動でチェックする手間を削減したい
- **エディタ非依存**: Neovim、VSCode、Zed、Helixなど、LSP対応エディタで共通して使える

### ビジネス価値
- **開発効率の向上**: バージョンチェックの自動化により開発者の時間を節約
- **セキュリティの向上**: 古いバージョンの早期発見により脆弱性リスクを低減
- **保守性の向上**: 依存関係を最新に保つことでメンテナンスコストを削減
- **エディタの選択自由**: LSP標準により、好みのエディタで使用可能

### 対象ユーザー
- LSP対応エディタのユーザー（Neovim、VSCode、Zed、Helix等）
- 複数の言語・ツールを使用する開発者
- 依存関係管理を効率化したい開発者

## 機能要件

### 必須機能（MUST have）

#### 1. パッケージファイルの解析
- tree-sitterを使用して以下のファイルを解析
  - `package.json` (JavaScript/TypeScript)
  - `Cargo.toml` (Rust)
  - `go.mod` (Go)
  - `.github/workflows/*.yml` (GitHub Actions)
- 各ファイルから依存パッケージとバージョン情報を抽出
- LSPの`textDocument/didOpen`、`textDocument/didChange`イベントで解析を実行

#### 2. バージョン管理機能
- 各言語の公式レジストリAPIから全バージョン一覧を取得
  - npm registry API (package.json)
  - crates.io API (Cargo.toml)
  - Go proxy API (go.mod)
  - GitHub Releases API (GitHub Actions)
- バージョン一覧をSQLiteデータベースに永続化
- LSP起動時に、TTLが過ぎたパッケージのバージョン一覧を更新
- 現在のバージョンと最新バージョンを比較
- セマンティックバージョニングによる比較（newer, latest, outdated, invalid）
- 存在しないバージョンの検出（DBに登録されていないバージョン）

#### 3. Diagnosticsによるバージョン情報の表示
- LSPの`textDocument/publishDiagnostics`でバージョン情報を通知
- 状態に応じたseverity
  - **最新**: `DiagnosticSeverity::Hint` - 情報表示のみ（表示するかは設定可能）
  - **古い**: `DiagnosticSeverity::Warning` - 警告として表示
  - **存在しない**: `DiagnosticSeverity::Error` - エラーとして表示
- メッセージ形式（英語）:
  - 最新: `"Latest version v1.2.3 available (current: v1.0.0)"`
  - エラー: `"Version v999.0.0 does not exist"`
- アイコンはエディタ側で設定可能

#### 4. LSP標準機能の実装
- **初期化**: `initialize` リクエストでサーバー能力を通知
- **ファイル監視**: `textDocument/didOpen`, `didChange`, `didClose`
- **非同期処理**: API呼び出しをノンブロッキングで実行
- **設定対応**: `workspace/didChangeConfiguration`で動的設定変更をサポート

#### 5. プラグイン拡張性
- パッケージタイプごとのモジュール化設計
- 新しいパッケージタイプを追加するためのトレイト定義
- `src/parsers/[package_type].rs` ファイルを追加するだけで新しいパッケージタイプをサポート

### オプション機能（NICE to have）

#### 1. バージョンキャッシュ
- SQLiteでバージョン情報をキャッシュ（永続化）
- 複数エディタ間でキャッシュを共有（同じユーザー、同じマシン）
- キャッシュファイル: `$XDG_CACHE_HOME/version-lsp/versions.db` または `~/.cache/version-lsp/versions.db`
- キャッシュ更新間隔: デフォルト24時間
- `version-lsp.cache.refresh_interval`設定で調整可能
- LSP起動時に自動更新（更新間隔を過ぎたパッケージのみ）
- API呼び出しの大幅削減（初回と更新間隔経過時のみ）

#### 2. 設定のカスタマイズ
- 各パッケージタイプの有効/無効切り替え
- チェック対象の依存関係種別（dependencies, devDependencies等）
- キャッシュ更新間隔（refresh_interval）
- 最新バージョンをHintとして表示するかどうか

#### 3. エラーハンドリング
- ネットワークエラー時の適切なメッセージ表示
- レート制限対応（Retry-Afterヘッダーの尊重）
- tree-sitterパーサー未インストール時のエラーメッセージ

### 将来の拡張性

#### 1. 追加パッケージタイプのサポート
- `requirements.txt` (Python)
- `Gemfile` (Ruby)
- `composer.json` (PHP)
- `pom.xml` (Java/Maven)
- `build.gradle` (Java/Gradle)

#### 2. Code Lens機能
- ファイル上部に「すべて最新」「n個のアップデートあり」を表示
- クリックで詳細を表示

#### 3. Code Action機能
- 「最新バージョンに更新」アクション
- ファイルの該当箇所を自動編集

## 非機能要件

### パフォーマンス要件
- tree-sitter解析: 100ms以内（通常サイズのファイル）
- LSP初期化: 500ms以内
- Diagnostics公開: API呼び出し完了後即座に（非同期）
- キャッシュヒット時: 10ms以内でDiagnosticsを公開
- 複数ファイル同時対応: 10ファイル程度まで同時処理可能

### セキュリティ要件
- API呼び出しは公式レジストリのみ（信頼できるソース）
- HTTPS通信の強制
- ユーザー入力のサニタイゼーション（パッケージ名、バージョン）
- コマンドインジェクション対策（外部コマンド実行時）

### 可用性・信頼性
- ネットワークエラー時もLSPサーバーがクラッシュしない
- API障害時も既存の機能を継続使用可能（キャッシュ利用）
- tree-sitterパーサー未インストール時の明確なエラーメッセージ
- パニック時のgraceful shutdown

### 保守性
- コードの可読性: 明確な関数・変数名、適切なコメント
- テスト容易性: 各モジュールの独立性を確保
- ドキュメント: API仕様、プラグイン拡張ガイド
- Rustのidiomatic patternに従う

## アーキテクチャ設計

### システム構成

```
version-lsp/
├── src/
│   ├── main.rs                          -- エントリポイント
│   ├── lsp/
│   │   ├── mod.rs                       -- LSPモジュールエントリポイント
│   │   ├── server.rs                    -- LSPサーバー起動・接続管理
│   │   ├── backend.rs                   -- LanguageServerトレイト実装
│   │   ├── handlers.rs                  -- LSPイベントハンドラー
│   │   └── diagnostics.rs               -- Diagnostics生成・公開
│   ├── parser/
│   │   ├── mod.rs                       -- パーサーモジュールエントリポイント
│   │   ├── traits.rs                    -- Parserトレイト定義
│   │   ├── types.rs                     -- 共通型定義（PackageInfo等）
│   │   ├── package_json.rs              -- package.json パーサー
│   │   ├── cargo_toml.rs                -- Cargo.toml パーサー
│   │   ├── go_mod.rs                    -- go.mod パーサー
│   │   └── github_actions.rs            -- GitHub Actions パーサー
│   ├── version/
│   │   ├── mod.rs                       -- バージョン管理モジュールエントリポイント
│   │   ├── checker.rs                   -- バージョンチェックロジック
│   │   ├── registry.rs                  -- Registryトレイト定義
│   │   ├── cache.rs                     -- SQLiteバージョンキャッシュ管理
│   │   ├── semver.rs                    -- セマンティックバージョン比較
│   │   ├── registries/
│   │   │   ├── mod.rs                   -- レジストリ実装エントリポイント
│   │   │   ├── npm.rs                   -- npm registry API
│   │   │   ├── crates_io.rs             -- crates.io API
│   │   │   ├── go_proxy.rs              -- Go proxy API
│   │   │   └── github.rs                -- GitHub Releases API
│   │   └── types.rs                     -- バージョン管理関連の型定義
│   └── config.rs                        -- グローバル設定管理
├── tests/
│   ├── parser/                          -- パーサーテスト
│   ├── version/                         -- バージョン管理テスト
│   └── integration/                     -- 統合テスト
├── docs/
│   ├── DESIGN.md                        -- 設計ドキュメント
│   ├── TODO.md                          -- タスクリスト
│   └── API.md                           -- プラグイン拡張API
└── Cargo.toml                           -- Rustプロジェクト設定
```

### レイヤーアーキテクチャ

#### 1. LSPプロトコル層（`lsp/`）
- `server.rs`: LSPサーバー起動、stdio接続管理
- `backend.rs`: `LanguageServer`トレイト実装
- `handlers.rs`: LSPイベントハンドラー（didOpen, didChange等）
- `diagnostics.rs`: Diagnostics生成・公開

#### 2. パーサー層（`parser/`）
- `traits.rs`: `Parser`トレイト定義
- `types.rs`: パーサー関連の型定義
- `*_parser.rs`: 各パッケージタイプの具体的なパーサー実装

#### 3. バージョン管理層（`version/`）
- `checker.rs`: バージョンチェックの統合ロジック
- `registry.rs`: `Registry`トレイト定義
- `cache.rs`: SQLiteバージョンキャッシュ管理
- `semver.rs`: セマンティックバージョン比較
- `registries/*.rs`: 各レジストリAPIの具体的な実装

#### 4. グローバル
- `config.rs`: 設定の管理とバリデーション

### モジュール間の依存関係

```
main.rs
  └─→ lsp/server.rs
        ├─→ version/cache.rs (初期化・更新間隔チェック・更新)
        │     └─→ version/registries/*.rs (バージョン一覧取得)
        └─→ lsp/backend.rs (LanguageServer)
              ├─→ lsp/handlers.rs
              │     ├─→ parser/mod.rs
              │     │     ├─→ parser/traits.rs
              │     │     └─→ parser/*_parser.rs
              │     └─→ version/checker.rs
              │           ├─→ version/cache.rs (バージョン存在チェック)
              │           └─→ version/semver.rs
              └─→ lsp/diagnostics.rs
```

### 技術スタック

- **言語**: Rust 1.70+ (2021 edition)
- **LSPフレームワーク**: tower-lsp 0.20+
- **パーサー**: tree-sitter 0.22+ (yaml, toml, json, go)
- **HTTP通信**: reqwest 0.12+ (async, rustls)
- **非同期ランタイム**: tokio 1.35+
- **シリアライゼーション**: serde, serde_json
- **セマンティックバージョン**: semver 1.0+
- **キャッシュストレージ**: sqlx 0.7+ (sqlite, runtime-tokio)
- **エラーハンドリング**: anyhow, thiserror

### パーサーモジュールの設計

各パーサーは共通のトレイトを実装：

```rust
use tower_lsp::lsp_types::{Position, Range};

/// パッケージ情報
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// パッケージ名 (例: "axios", "tokio")
    pub name: String,
    /// 現在のバージョン (例: "1.2.3", "^1.0.0")
    pub version: String,
    /// ファイル内の位置
    pub range: Range,
    /// レジストリタイプ
    pub registry_type: RegistryType,
}

/// レジストリタイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryType {
    Npm,
    Crates,
    Go,
    GitHub,
}

/// パーサートレイト
#[async_trait::async_trait]
pub trait Parser: Send + Sync {
    /// ファイルがこのパーサーで処理可能かチェック
    fn can_parse(&self, uri: &str) -> bool;

    /// ファイルを解析してパッケージ情報を抽出
    async fn parse(&self, content: &str) -> Result<Vec<PackageInfo>>;
}
```

### レジストリモジュールの設計

各レジストリAPIは共通のトレイトを実装：

```rust
/// パッケージの全バージョン情報
#[derive(Debug, Clone)]
pub struct PackageVersions {
    /// パッケージ名
    pub package_name: String,
    /// 全バージョンのリスト（新しい順）
    pub versions: Vec<String>,
}

/// レジストリトレイト
#[async_trait::async_trait]
pub trait Registry: Send + Sync {
    /// 全バージョン一覧を取得（非同期）
    async fn fetch_all_versions(&self, package_name: &str) -> Result<PackageVersions>;

    /// レジストリタイプを返す
    fn registry_type(&self) -> RegistryType;
}
```

## データ設計

### パッケージ情報のデータモデル

```rust
/// パッケージ情報
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub range: Range,
    pub registry_type: RegistryType,
}

/// バージョン比較結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionStatus {
    /// 最新バージョンを使用中
    Latest,
    /// 古いバージョンを使用中
    Outdated { latest: String },
    /// 現在のバージョンがより新しい（異常）
    Newer { latest: String },
    /// バージョンが存在しない
    NotFound,
    /// バージョン形式が無効
    Invalid,
}

/// チェック結果
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub package: PackageInfo,
    pub status: VersionStatus,
}
```

### バージョンキャッシュ構造（SQLite）

```sql
-- パッケージメタデータテーブル
CREATE TABLE IF NOT EXISTS packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    registry_type TEXT NOT NULL,      -- "npm", "crates", "go", "github"
    package_name TEXT NOT NULL,       -- パッケージ名
    updated_at INTEGER NOT NULL,      -- UNIX timestamp (秒)
    UNIQUE(registry_type, package_name)
);

CREATE INDEX IF NOT EXISTS idx_updated_at ON packages(updated_at);
CREATE INDEX IF NOT EXISTS idx_registry_package ON packages(registry_type, package_name);

-- バージョンテーブル
CREATE TABLE IF NOT EXISTS versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,      -- packages.id への外部キー
    version TEXT NOT NULL,            -- バージョン文字列 (例: "1.2.3")
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    UNIQUE(package_id, version)
);

CREATE INDEX IF NOT EXISTS idx_package_id ON versions(package_id);
```

```rust
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

/// パッケージメタデータ
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub id: i64,
    pub registry_type: String,
    pub package_name: String,
    pub updated_at: i64,  // UNIX timestamp
}

/// SQLiteバージョンキャッシュ
pub struct Cache {
    pool: SqlitePool,
    refresh_interval: i64,  // seconds
}

impl Cache {
    /// キャッシュを初期化（$XDG_CACHE_HOME/version-lsp/versions.db）
    pub async fn new(refresh_interval: i64) -> Result<Self>;

    /// LSP起動時の初期化：更新間隔を過ぎたパッケージを更新
    pub async fn initialize(&self, registries: &[Box<dyn Registry>]) -> Result<()>;

    /// パッケージの全バージョンを取得
    pub async fn get_versions(&self, registry_type: &str, package_name: &str) -> Result<Vec<String>>;

    /// パッケージとバージョン一覧を保存
    pub async fn save_versions(&self, registry_type: &str, package_name: &str, versions: Vec<String>) -> Result<()>;

    /// バージョンが存在するかチェック
    pub async fn version_exists(&self, registry_type: &str, package_name: &str, version: &str) -> Result<bool>;

    /// 更新間隔を過ぎたパッケージのリストを取得
    pub async fn get_stale_packages(&self) -> Result<Vec<(String, String)>>;  // (registry_type, package_name)

    /// 最新バージョンを取得
    pub async fn get_latest_version(&self, registry_type: &str, package_name: &str) -> Result<Option<String>>;
}
```

## LSP機能設計

### 0. 起動時の初期化フロー

LSPサーバー起動時に以下の処理を実行：

```rust
async fn initialize_lsp() -> Result<()> {
    // 1. キャッシュを開く/作成
    let cache = Cache::new(refresh_interval_seconds).await?;

    // 2. 更新間隔を過ぎたパッケージを取得
    let stale_packages = cache.get_stale_packages().await?;

    // 3. バックグラウンドで更新（ノンブロッキング）
    tokio::spawn(async move {
        for (registry_type, package_name) in stale_packages {
            // レジストリから全バージョンを取得
            let registry = get_registry(&registry_type);
            match registry.fetch_all_versions(&package_name).await {
                Ok(package_versions) => {
                    // キャッシュに保存
                    cache.save_versions(&registry_type, &package_name, package_versions.versions).await?;
                }
                Err(e) => {
                    eprintln!("Failed to update {}: {}", package_name, e);
                }
            }
        }
        Ok::<(), Error>(())
    });

    // 4. LSPサーバーを起動（更新を待たずに起動）
    Ok(())
}
```

**設計のポイント:**
- LSPサーバー起動を遅延させない（バックグラウンド更新）
- 初回起動時はキャッシュが空なので、ファイルを開いた時にオンデマンドで取得
- 2回目以降の起動では、更新間隔を過ぎたパッケージのみを更新

### 1. サーバー能力（ServerCapabilities）

```json
{
  "textDocumentSync": {
    "openClose": true,
    "change": 2  // Incremental
  },
  "diagnosticProvider": {
    "interFileDependencies": false,
    "workspaceDiagnostics": false
  }
}
```

### 2. Diagnostics仕様

#### 最新バージョンの場合（オプション、デフォルトOFF）
```json
{
  "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 5, "character": 20 } },
  "severity": 4,  // Hint
  "message": "Using latest version v1.2.3",
  "source": "version-lsp"
}
```

#### 古いバージョンの場合
```json
{
  "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 5, "character": 20 } },
  "severity": 2,  // Warning
  "message": "Latest version v1.2.3 available (current: v1.0.0)",
  "source": "version-lsp"
}
```

#### 存在しないバージョンの場合
```json
{
  "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 15 } },
  "severity": 1,  // Error
  "message": "Version v999.0.0 does not exist",
  "source": "version-lsp"
}
```

### 3. 設定（Configuration）

クライアントから`workspace/configuration`で取得：

```json
{
  "version-lsp": {
    "enabled": true,
    "cache": {
      "refresh_interval": 86400  // 秒（デフォルト24時間）
    },
    "diagnostics": {
      "showLatest": false,  // 最新バージョンをHintとして表示するか
      "enabled": {
        "npm": true,
        "crates": true,
        "go": true,
        "github": true
      }
    },
    "npm": {
      "checkDependencies": true,
      "checkDevDependencies": true,
      "checkPeerDependencies": false
    }
  }
}
```

## セキュリティ設計

### 入力検証
- パッケージ名: 英数字、ハイフン、アンダースコア、スラッシュ、ドット、@のみ許可
- バージョン文字列: セマンティックバージョニング形式の検証（`semver`クレート使用）
- URI検証: `file://`スキームのみ許可

### API呼び出しのセキュリティ
- HTTPS通信の強制（reqwest with rustls）
- 公式レジストリのドメイン検証
  - `registry.npmjs.org`
  - `crates.io`
  - `proxy.golang.org`
  - `api.github.com`
- タイムアウト設定（30秒）
- User-Agentヘッダーの設定（`version-lsp/x.y.z`）

### エラーハンドリング
- `Result<T, E>`型で全エラーを処理
- パニックを避け、`?`演算子とエラー伝播を活用
- クライアントへのエラー通知（`window/showMessage`）

## パフォーマンス設計

### 最適化戦略

#### バージョンキャッシュ
- **SQLiteキャッシュ**: 全バージョン情報をSQLiteに永続化
- **キャッシュ場所**: `$XDG_CACHE_HOME/version-lsp/versions.db` または `~/.cache/version-lsp/versions.db`
- **複数エディタ共有**: 同じユーザー・同じマシン上の複数エディタでキャッシュを共有
- **更新間隔**: デフォルト24時間（設定可能）
- **API呼び出し削減**: ファイルを開くたびのAPI呼び出しが不要（キャッシュから取得）
- **並行アクセス**: SQLiteのWALモードで安全な並行読み書き
- **バックグラウンド更新**: LSP起動時に更新間隔を過ぎたパッケージをバックグラウンドで更新
- **オンデマンド取得**: 初回アクセス時にキャッシュにないパッケージは即座に取得・保存

#### 非同期処理
- **API呼び出し**: `reqwest`で非同期実行
- **複数パッケージ**: `tokio::spawn`で並列処理
- **Diagnostics公開**: 各パッケージのチェック完了後、即座に公開

#### tree-sitter最適化
- **必要な範囲のみ解析**: 依存関係セクションに焦点を当てる
- **インクリメンタル更新**: `textDocument/didChange`でインクリメンタルパース（将来的）

### スケーラビリティ
- **大量パッケージ対応**: 100個以上のパッケージでもスムーズに動作
- **並列API呼び出し**: `tokio::spawn`で最大10並列
- **メモリ効率**: 不要なデータのクローンを避ける

## 開発・運用

### 開発環境
- Rust 1.70+ (2021 edition)
- tree-sitter CLI
- SQLite 3.35+ (WALモード対応、バージョンキャッシュ用)
- 必要なtree-sitterパーサー:
  - `tree-sitter-yaml`
  - `tree-sitter-toml`
  - `tree-sitter-json`
  - `tree-sitter-go`

### ビルドとインストール
```bash
# 開発ビルド
cargo build

# リリースビルド
cargo build --release

# インストール
cargo install --path .
```

### テスト戦略
- **単体テスト**: 各パーサー、各レジストリAPIの動作検証
- **統合テスト**: 実際のファイルでの動作確認
- **モックAPI**: テスト時は実際のAPIを呼ばない（`mockito`使用）
- **テストカバレッジ**: `cargo-tarpaulin`で80%以上を目標

### CI/CD
- GitHub Actions
- 静的解析: `clippy`, `rustfmt`
- テストの自動実行: `cargo test`
- リリース: GitHub Releasesでバイナリ配布

## エディタ設定

### Neovim設定例

```lua
-- nvim-lspconfigを使用
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

-- version-lspの設定を追加
if not configs.version_lsp then
  configs.version_lsp = {
    default_config = {
      cmd = { 'version-lsp' },
      filetypes = { 'json', 'toml', 'yaml', 'go' },
      root_dir = lspconfig.util.root_pattern('.git', 'package.json', 'Cargo.toml', 'go.mod'),
      settings = {
        ['version-lsp'] = {
          cache = { refresh_interval = 86400 },  -- 24時間
          diagnostics = { showLatest = false }
        }
      }
    }
  }
end

-- LSPを起動
lspconfig.version_lsp.setup{}

-- Diagnostics表示のカスタマイズ
vim.diagnostic.config({
  virtual_text = {
    prefix = '',
    format = function(diagnostic)
      return diagnostic.message
    end,
  },
  signs = true,
  underline = true,
})

-- Diagnosticsのアイコン設定
local signs = { Error = "✗", Warn = "⚠", Hint = "✓", Info = "ℹ" }
for type, icon in pairs(signs) do
  local hl = "DiagnosticSign" .. type
  vim.fn.sign_define(hl, { text = icon, texthl = hl, numhl = hl })
end
```

### VSCode設定例

```json
{
  "version-lsp.enable": true,
  "version-lsp.cache.refresh_interval": 86400,
  "version-lsp.diagnostics.showLatest": false
}
```

**注**: VSCode用の拡張機能は別途必要（将来的に開発）

## 制約と前提

### 技術的制約
- tree-sitterパーサーが必須（yaml, toml, json, go）
- SQLite 3.35+が必要（バージョンキャッシュ）
- インターネット接続が必要（API呼び出し、初回と更新間隔経過時）
- LSP対応エディタが必要（Neovim 0.9+、VSCode等）
- 書き込み可能なキャッシュディレクトリが必要（`$XDG_CACHE_HOME`または`~/.cache`）

### ビジネス制約
- 個人開発プロジェクト（1人）
- オープンソース（MIT License想定）

### 依存関係
- tower-lsp（LSPフレームワーク）
- tree-sitter（ファイル解析）
- reqwest（HTTP通信）
- tokio（非同期ランタイム）
- sqlx（SQLiteバージョンキャッシュ）
- 各レジストリの公式API（無料枠の制限あり）

## 実装の優先順位

### Phase 1: MVP（最小機能）
1. LSPサーバーの骨格（tower-lsp）
2. SQLiteバージョンキャッシュ（スキーマ、CRUD操作）
3. 起動時の初期化処理（更新間隔を過ぎたパッケージの更新）
4. GitHub Actions パーサー（参照実装ベース）
5. GitHub Releases API（全バージョン一覧取得）
6. バージョンチェック機能（存在チェック、最新チェック）
7. Diagnostics公開

### Phase 2: 主要パッケージタイプ対応
1. package.json パーサー + npm registry API
2. Cargo.toml パーサー + crates.io API
3. go.mod パーサー + Go proxy API

### Phase 3: 設定とエラーハンドリング
1. 設定の読み込み（`workspace/configuration`）
2. エラーハンドリングの強化
3. ログ機能（`window/logMessage`）

### Phase 4: 拡張性とドキュメント
1. プラグイン拡張API
2. ドキュメント整備（README, API.md）
3. エディタ設定例

## 参照
- タスク分解: task-planning スキルでTODO.mdを生成
- 参照実装: `/Users/skanehira/dev/github.com/skanehira/github-actions.nvim/lua/github-actions/versions`
- TDD準拠: すべての機能はテストファーストで実装
- LSP仕様: https://microsoft.github.io/language-server-protocol/
- tower-lsp: https://github.com/ebkalderon/tower-lsp
