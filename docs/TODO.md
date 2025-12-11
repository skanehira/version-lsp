# TODO: version-lsp

作成日: 2025-12-03
生成元: task-planning
設計書: docs/DESIGN.md

## 概要

複数の言語・ツールのパッケージ管理ファイルに記載されているパッケージバージョンをチェックし、最新バージョンが利用可能な場合にLSPを通じてエディタ上でリアルタイムに表示するLSPサーバー。

**目標**: エディタ非依存（Neovim、VSCode、Zed、Helix等）で動作するLSPサーバーの実装。

## 実装タスク

### Phase 0: プロジェクト基盤

#### 0.1 プロジェクト構造のセットアップ

- [x] [STRUCTURAL] Cargo.tomlの作成と依存関係の定義
  - tower-lsp 0.20+
  - tree-sitter 0.22+
  - reqwest 0.12+ (async, rustls)
  - tokio 1.35+
  - serde, serde_json
  - semver 1.0+
  - sqlx 0.7+ (sqlite, runtime-tokio)
  - anyhow, thiserror
  - async-trait

- [x] [STRUCTURAL] ディレクトリ構造の作成
  - `src/lsp/` (LSPプロトコル層)
  - `src/parser/` (パーサー層)
  - `src/version/` (バージョン管理層)
  - `tests/parser/` (パーサーテスト)
  - `tests/version/` (バージョン管理テスト)
  - `tests/integration/` (統合テスト)

- [x] [STRUCTURAL] .gitignoreとCIの設定
  - Rust標準の.gitignore
  - GitHub Actions設定（clippy, rustfmt, test）

---

### Phase 1: バージョンキャッシュ（基盤）

#### 1.1 SQLiteスキーマとマイグレーション

- [x] [RED] キャッシュ初期化のテスト作成 (`tests/version/cache_test.rs`)
  - `Cache::new()`でDBファイルが作成されることを確認
  - テーブル（packages, versions）が存在することを確認

- [x] [GREEN] Cache構造体とスキーマ実装 (`src/version/cache.rs`)
  - `Cache::new(refresh_interval: i64) -> Result<Self>`
  - CREATE TABLE文の実行
  - INDEXの作成

- [x] [REFACTOR] エラーハンドリングとログ追加
  - thiserrorでカスタムエラー型定義
  - ログ出力の追加

#### 1.2 バージョン保存機能

- [x] [RED] バージョン保存のテスト作成
  - `save_versions()`で新規パッケージを保存できることを確認
  - 既存パッケージの更新ができることを確認（updated_atの更新）

- [x] [GREEN] バージョン保存の実装
  - `save_versions(registry_type, package_name, versions) -> Result<()>`
  - トランザクション処理（パッケージ作成/更新 + バージョン一括挿入）
  - 既存バージョンの削除と再挿入

- [x] [REFACTOR] SQLクエリの最適化
  - バルクインサートの効率化
  - エラーメッセージの改善

#### 1.3 バージョン取得機能

- [x] [RED] バージョン取得のテスト作成
  - `get_versions()`で保存済みバージョンを取得できることを確認
  - 存在しないパッケージの場合は空配列を返すことを確認

- [x] [GREEN] バージョン取得の実装
  - `get_versions(registry_type, package_name) -> Result<Vec<String>>`
  - JOINクエリでpackagesとversionsを結合

- [x] [REFACTOR] クエリのパフォーマンステスト
  - 1000件のバージョンでも10ms以内で取得できることを確認

#### 1.4 バージョン存在チェック

- [x] [RED] バージョン存在チェックのテスト作成
  - `version_exists()`で存在するバージョンに対してtrueを返すことを確認
  - 存在しないバージョンに対してfalseを返すことを確認

- [x] [GREEN] バージョン存在チェックの実装
  - `version_exists(registry_type, package_name, version) -> Result<bool>`
  - EXISTSクエリでの効率的なチェック

#### 1.5 最新バージョン取得

- [x] [RED] 最新バージョン取得のテスト作成
  - `get_latest_version()`で最新バージョンを取得できることを確認
  - versionsリストの最初の要素を返すことを確認

- [x] [GREEN] 最新バージョン取得の実装
  - `get_latest_version(registry_type, package_name) -> Result<Option<String>>`
  - LIMIT 1クエリでの取得

#### 1.6 古いパッケージの取得

- [x] [RED] 古いパッケージ取得のテスト作成
  - `get_stale_packages()`で更新間隔を過ぎたパッケージを取得できることを確認
  - 現在時刻と`refresh_interval`を使った判定

- [x] [GREEN] 古いパッケージ取得の実装
  - `get_stale_packages() -> Result<Vec<(String, String)>>`
  - `WHERE updated_at < (現在時刻 - refresh_interval)`

---

### Phase 2: セマンティックバージョン比較

#### 2.1 バージョン比較ロジック

- [ ] [RED] セマンティックバージョン比較のテスト作成 (`tests/version/semver_test.rs`)
  - `compare_versions()`でlatest/outdated/newerを判定できることを確認
  - エッジケース（prereleaseバージョン、メタデータ付きなど）のテスト

- [ ] [GREEN] バージョン比較の実装 (`src/version/semver.rs`)
  - `semver`クレートを使用
  - `VersionStatus`列挙型の定義
  - `compare_versions(current, latest) -> VersionStatus`

- [ ] [REFACTOR] エラーハンドリング
  - 無効なバージョン形式の場合は`VersionStatus::Invalid`を返す

---

### Phase 3: GitHub Actionsパーサー

#### 3.1 Parserトレイトの定義

- [ ] [STRUCTURAL] Parserトレイトの定義 (`src/parser/traits.rs`)
  - `Parser`トレイト定義
  - `can_parse(&self, uri: &str) -> bool`
  - `parse(&self, content: &str) -> Result<Vec<PackageInfo>>`

- [ ] [STRUCTURAL] 共通型の定義 (`src/parser/types.rs`)
  - `PackageInfo`構造体
  - `RegistryType`列挙型

#### 3.2 GitHub Actionsパーサーの実装

- [ ] [RED] GitHub Actionsパーサーのテスト作成 (`tests/parser/github_actions_test.rs`)
  - workflowファイルから`uses:`を抽出できることを確認
  - `actions/checkout@v3`のようなアクション参照をパースできることを確認
  - バージョンタグとハッシュの両方に対応

- [ ] [GREEN] GitHub Actionsパーサーの実装 (`src/parser/github_actions.rs`)
  - tree-sitter-yamlを使用
  - `uses:`フィールドの抽出
  - `owner/repo@version`形式のパース
  - 参照実装（`/Users/skanehira/dev/github.com/skanehira/github-actions.nvim/lua/github-actions/versions/parser.lua`）を参考

- [ ] [REFACTOR] エラーハンドリングとログ
  - tree-sitterパーサー未インストール時のエラーメッセージ
  - 不正な形式のワークフローファイルの処理

---

### Phase 4: GitHub Releases API

#### 4.1 Registryトレイトの定義

- [ ] [STRUCTURAL] Registryトレイトの定義 (`src/version/registry.rs`)
  - `Registry`トレイト定義
  - `fetch_all_versions(&self, package_name: &str) -> Result<PackageVersions>`
  - `registry_type(&self) -> RegistryType`

- [ ] [STRUCTURAL] 共通型の定義 (`src/version/types.rs`)
  - `PackageVersions`構造体

#### 4.2 GitHub Releases APIの実装

- [ ] [RED] GitHub Releases APIのテスト作成 (`tests/version/github_test.rs`)
  - モックAPIサーバー（mockito）でテスト
  - `fetch_all_versions()`で全リリースを取得できることを確認
  - タグ名からバージョンを抽出（`v1.2.3` → `1.2.3`）

- [ ] [GREEN] GitHub Releases APIの実装 (`src/version/registries/github.rs`)
  - reqwestでGitHub API (`https://api.github.com/repos/{owner}/{repo}/releases`)を呼び出し
  - タグ名のリストを取得
  - 新しい順にソート

- [ ] [REFACTOR] エラーハンドリング
  - ネットワークエラー
  - レート制限（429 Too Many Requests）
  - 存在しないリポジトリ（404 Not Found）

---

### Phase 5: バージョンチェッカー

#### 5.1 チェック統合ロジック

- [ ] [RED] バージョンチェッカーのテスト作成 (`tests/version/checker_test.rs`)
  - `check_version()`でキャッシュから最新バージョンを取得し、現在のバージョンと比較
  - `CheckResult`を返すことを確認

- [ ] [GREEN] バージョンチェッカーの実装 (`src/version/checker.rs`)
  - `check_version(cache, package_info) -> Result<CheckResult>`
  - キャッシュからバージョン一覧を取得
  - `version_exists()`で存在チェック
  - `get_latest_version()`で最新バージョン取得
  - `compare_versions()`でバージョン比較

- [ ] [REFACTOR] 非同期処理の最適化
  - 複数パッケージの並列チェック

---

### Phase 6: LSPサーバー骨格

#### 6.1 LSPサーバー起動

- [ ] [RED] LSPサーバー起動のテスト作成 (`tests/integration/lsp_test.rs`)
  - `initialize`リクエストでServerCapabilitiesを返すことを確認

- [ ] [GREEN] LSPサーバーの実装 (`src/lsp/server.rs`, `src/lsp/backend.rs`)
  - `main.rs`でサーバー起動
  - tower-lspの`LanguageServer`トレイト実装
  - `initialize()`でServerCapabilitiesを返す

- [ ] [REFACTOR] ロギングとエラーハンドリング
  - 起動ログ
  - パニック時のgraceful shutdown

#### 6.2 キャッシュの初期化

- [ ] [RED] LSP起動時のキャッシュ初期化テスト作成
  - 起動時に`get_stale_packages()`が呼ばれることを確認
  - バックグラウンドタスクが起動されることを確認

- [ ] [GREEN] キャッシュ初期化の実装
  - サーバー起動時に`Cache::new()`を呼び出し
  - `tokio::spawn()`でバックグラウンド更新タスクを起動
  - `get_stale_packages()`で古いパッケージを取得し、レジストリから更新

- [ ] [REFACTOR] エラーログとメトリクス
  - 更新失敗時のログ
  - 更新にかかった時間のログ

---

### Phase 7: Diagnostics生成と公開

#### 7.1 Diagnostics生成

- [ ] [RED] Diagnostics生成のテスト作成 (`tests/lsp/diagnostics_test.rs`)
  - `CheckResult`から`Diagnostic`を生成できることを確認
  - メッセージが正しい形式（英語）であることを確認

- [ ] [GREEN] Diagnostics生成の実装 (`src/lsp/diagnostics.rs`)
  - `create_diagnostic(check_result) -> Diagnostic`
  - severityの決定（Latest→Hint, Outdated→Warning, NotFound→Error）
  - メッセージの生成（"Latest version v1.2.3 available (current: v1.0.0)"）

#### 7.2 ファイル監視とDiagnostics公開

- [ ] [RED] ファイル監視のテスト作成
  - `textDocument/didOpen`でパースが実行されることを確認
  - `textDocument/publishDiagnostics`が送信されることを確認

- [ ] [GREEN] ファイル監視の実装 (`src/lsp/handlers.rs`)
  - `textDocument/didOpen`ハンドラー
  - ファイル内容のパース
  - バージョンチェック
  - Diagnosticsの公開

- [ ] [REFACTOR] 非同期処理とエラーハンドリング
  - パース失敗時の処理
  - API呼び出し失敗時の処理

---

### Phase 8: 統合テスト

#### 8.1 E2Eテスト

- [ ] [RED] E2Eテストの作成 (`tests/integration/e2e_test.rs`)
  - 実際のworkflowファイルを開く
  - Diagnosticsが正しく返されることを確認

- [ ] [GREEN] テストフィクスチャの準備
  - サンプルワークフローファイル（`.github/workflows/test.yml`）
  - モックレジストリレスポンス

- [ ] [REFACTOR] テストの安定化
  - タイムアウト設定
  - リトライロジック

---

### Phase 9: package.jsonパーサー

#### 9.1 package.jsonパーサーの実装

- [ ] [RED] package.jsonパーサーのテスト作成 (`tests/parser/package_json_test.rs`)
  - `dependencies`、`devDependencies`を抽出できることを確認
  - バージョン範囲（`^1.0.0`、`~1.0.0`等）のパース

- [ ] [GREEN] package.jsonパーサーの実装 (`src/parser/package_json.rs`)
  - tree-sitter-jsonを使用
  - `dependencies`、`devDependencies`、`peerDependencies`の抽出

- [ ] [REFACTOR] バージョン範囲の正規化
  - `^1.0.0` → `1.0.0`（比較用）

---

### Phase 10: npm registry API

#### 10.1 npm registry APIの実装

- [ ] [RED] npm registry APIのテスト作成 (`tests/version/npm_test.rs`)
  - モックAPIサーバーでテスト
  - `fetch_all_versions()`で全バージョンを取得できることを確認

- [ ] [GREEN] npm registry APIの実装 (`src/version/registries/npm.rs`)
  - reqwestでnpm registry (`https://registry.npmjs.org/{package}`)を呼び出し
  - `versions`フィールドから全バージョンを抽出

- [ ] [REFACTOR] エラーハンドリング
  - 存在しないパッケージ（404）
  - scoped packages (`@types/node`)の対応

---

### Phase 11: Cargo.tomlパーサー

#### 11.1 Cargo.tomlパーサーの実装

- [ ] [RED] Cargo.tomlパーサーのテスト作成 (`tests/parser/cargo_toml_test.rs`)
  - `[dependencies]`、`[dev-dependencies]`を抽出できることを確認

- [ ] [GREEN] Cargo.tomlパーサーの実装 (`src/parser/cargo_toml.rs`)
  - tree-sitter-tomlを使用
  - `[dependencies]`、`[dev-dependencies]`、`[build-dependencies]`の抽出

- [ ] [REFACTOR] バージョン指定の対応
  - `version = "1.0"`
  - `{ version = "1.0", features = ["..."] }`

---

### Phase 12: crates.io API

#### 12.1 crates.io APIの実装

- [ ] [RED] crates.io APIのテスト作成 (`tests/version/crates_io_test.rs`)
  - モックAPIサーバーでテスト
  - `fetch_all_versions()`で全バージョンを取得できることを確認

- [ ] [GREEN] crates.io APIの実装 (`src/version/registries/crates_io.rs`)
  - reqwestでcrates.io API (`https://crates.io/api/v1/crates/{crate}`)を呼び出し
  - `versions`配列から全バージョンを抽出

- [ ] [REFACTOR] エラーハンドリング
  - 存在しないクレート（404）

---

### Phase 13: go.modパーサー

#### 13.1 go.modパーサーの実装

- [ ] [RED] go.modパーサーのテスト作成 (`tests/parser/go_mod_test.rs`)
  - `require`ディレクティブを抽出できることを確認

- [ ] [GREEN] go.modパーサーの実装 (`src/parser/go_mod.rs`)
  - tree-sitter-goを使用
  - `require`ディレクティブの抽出

- [ ] [REFACTOR] バージョン形式の対応
  - `v1.2.3`
  - `v1.2.3+incompatible`
  - pseudo-versionsの対応

---

### Phase 14: Go proxy API

#### 14.1 Go proxy APIの実装

- [ ] [RED] Go proxy APIのテスト作成 (`tests/version/go_proxy_test.rs`)
  - モックAPIサーバーでテスト
  - `fetch_all_versions()`で全バージョンを取得できることを確認

- [ ] [GREEN] Go proxy APIの実装 (`src/version/registries/go_proxy.rs`)
  - reqwestでGo proxy (`https://proxy.golang.org/{module}/@v/list`)を呼び出し
  - バージョンリストを取得

- [ ] [REFACTOR] エラーハンドリング
  - 存在しないモジュール（404/410）

---

### Phase 15: 設定とエラーハンドリング

#### 15.1 設定の読み込み

- [ ] [RED] 設定読み込みのテスト作成 (`tests/integration/config_test.rs`)
  - `workspace/configuration`でクライアントから設定を取得できることを確認

- [ ] [GREEN] 設定の実装 (`src/config.rs`)
  - `workspace/didChangeConfiguration`ハンドラー
  - 設定構造体の定義
  - デフォルト値の設定

- [ ] [REFACTOR] 設定のバリデーション
  - `refresh_interval`の範囲チェック（1時間〜7日）

#### 15.2 エラーハンドリングの強化

- [ ] [REFACTOR] 全モジュールのエラーハンドリングレビュー
  - パニックの削除
  - `Result<T, E>`の一貫した使用
  - クライアントへの適切なエラー通知（`window/showMessage`）

---

### Phase 16: ドキュメントとCI/CD

#### 16.1 ドキュメント整備

- [ ] [STRUCTURAL] README.mdの作成
  - インストール方法
  - エディタ設定例（Neovim、VSCode）
  - 対応パッケージファイル一覧

- [ ] [STRUCTURAL] API.mdの作成
  - プラグイン拡張API
  - 新しいパーサーの追加方法

#### 16.2 CI/CDの設定

- [ ] [STRUCTURAL] GitHub Actionsワークフローの作成
  - clippy
  - rustfmt
  - cargo test
  - cargo build --release

- [ ] [STRUCTURAL] リリースワークフローの作成
  - バイナリのビルド（Linux、macOS、Windows）
  - GitHub Releasesへのアップロード

---

## 実装ノート

### MUSTルール遵守事項

#### TDD: RED → GREEN → REFACTOR サイクルを厳守
- 各機能でテストを先に書く（RED）
- 最小限の実装でテストを通す（GREEN）
- コード品質を改善（REFACTOR）

#### Tidy First: 構造変更と動作変更を分離
- [STRUCTURAL]: コード整理のみ（動作変更なし）
- [BEHAVIORAL]: 機能追加・変更（[RED]/[GREEN]/[REFACTOR]）

#### コミット規律
- テストが全て合格してからコミット
- コミットメッセージに`[STRUCTURAL]`または`[BEHAVIORAL]`プレフィックスを付ける
- 例: `[BEHAVIORAL] [GREEN] Implement GitHub Actions parser`

### 参照ドキュメント
- 設計書: `docs/DESIGN.md`
- 参照実装: `/Users/skanehira/dev/github.com/skanehira/github-actions.nvim/lua/github-actions/versions`
- LSP仕様: https://microsoft.github.io/language-server-protocol/
- tower-lsp: https://github.com/ebkalderon/tower-lsp

### テスト戦略
- 単体テスト: 各モジュールの動作検証
- 統合テスト: LSP全体の動作確認
- モックAPI: テスト時は実際のAPIを呼ばない（mockito使用）
- カバレッジ目標: 80%以上

### 開発環境
- Rust 1.70+
- tree-sitter CLI
- SQLite 3.35+
- 必要なtree-sitterパーサー: yaml, toml, json, go
