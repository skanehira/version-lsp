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

- [x] [RED] 更新が必要なパッケージ取得のテスト作成
  - `get_packages_needing_refresh()`で更新間隔を過ぎたパッケージを取得できることを確認
  - 現在時刻と`refresh_interval`を使った判定

- [x] [GREEN] 更新が必要なパッケージ取得の実装
  - `get_packages_needing_refresh() -> Result<Vec<(String, String)>>`
  - `WHERE updated_at < (現在時刻 - refresh_interval)`

---

### Phase 2: セマンティックバージョン比較

#### 2.1 バージョン比較ロジック

- [x] [RED] セマンティックバージョン比較のテスト作成 (`src/version/semver.rs`内の`#[cfg(test)]`)
  - `compare_versions()`でlatest/outdated/newerを判定できることを確認
  - エッジケース（prereleaseバージョン、メタデータ付きなど）のテスト

- [x] [GREEN] バージョン比較の実装 (`src/version/semver.rs`)
  - `semver`クレートを使用
  - `VersionStatus`列挙型の定義
  - `compare_versions(current, latest) -> VersionStatus`

- [x] [REFACTOR] エラーハンドリング
  - 無効なバージョン形式の場合は`VersionStatus::Invalid`を返す

---

### Phase 3: GitHub Actionsパーサー

#### 3.1 Parserトレイトの定義

- [x] [STRUCTURAL] Parserトレイトの定義 (`src/parser/traits.rs`)
  - `Parser`トレイト定義
  - `can_parse(&self, uri: &str) -> bool`
  - `parse(&self, content: &str) -> Result<Vec<PackageInfo>>`

- [x] [STRUCTURAL] 共通型の定義 (`src/parser/types.rs`)
  - `PackageInfo`構造体
  - `RegistryType`列挙型

#### 3.2 GitHub Actionsパーサーの実装

- [x] [RED] GitHub Actionsパーサーのテスト作成 (`src/parser/github_actions.rs`内の`#[cfg(test)]`)
  - workflowファイルから`uses:`を抽出できることを確認
  - `actions/checkout@v3`のようなアクション参照をパースできることを確認
  - バージョンタグとハッシュの両方に対応
  - steps外のuses（jobレベル、workflow_call）を無視
  - ハッシュ+コメントバージョン（`@hash # v1.2.3`）をサポート

- [x] [GREEN] GitHub Actionsパーサーの実装 (`src/parser/github_actions.rs`)
  - tree-sitter-yamlを使用
  - `uses:`フィールドの抽出（steps内のみ）
  - `owner/repo@version`形式のパース
  - コミットハッシュ検出とコメントからのバージョン抽出
  - 参照実装（`/Users/skanehira/dev/github.com/skanehira/github-actions.nvim/lua/github-actions/versions/parser.lua`）を参考

- [x] [REFACTOR] エラーハンドリングとログ
  - tree-sitterパーサー未インストール時のエラーメッセージ
  - 不正な形式のワークフローファイルの処理

---

### Phase 4: GitHub Releases API ✅

#### 4.1 Registryトレイトの定義

- [x] [STRUCTURAL] Registryトレイトの定義 (`src/version/registry.rs`)
  - `Registry`トレイト定義
  - `fetch_all_versions(&self, package_name: &str) -> Result<PackageVersions>`
  - `registry_type(&self) -> RegistryType`

- [x] [STRUCTURAL] 共通型の定義 (`src/version/types.rs`)
  - `PackageVersions`構造体
  - `RegistryError`エラー型 (`src/version/error.rs`)

#### 4.2 GitHub Releases APIの実装

- [x] [RED] GitHub Releases APIのテスト作成 (`src/version/registries/github.rs`内の`#[cfg(test)]`)
  - モックAPIサーバー（mockito）でテスト
  - `fetch_all_versions()`で全リリースを取得できることを確認
  - 空リリース、404、429のテストケース

- [x] [GREEN] GitHub Releases APIの実装 (`src/version/registries/github.rs`)
  - reqwestでGitHub API (`https://api.github.com/repos/{owner}/{repo}/releases`)を呼び出し
  - タグ名のリストを取得
  - APIレスポンスからバージョン抽出

- [x] [REFACTOR] エラーハンドリング
  - ネットワークエラー (`RegistryError::Network`)
  - レート制限 (`RegistryError::RateLimited` with retry-after)
  - 存在しないリポジトリ (`RegistryError::NotFound`)
  - 不正なレスポンス (`RegistryError::InvalidResponse`)

---

### Phase 5: バージョンチェッカー ✅

#### 5.1 チェック統合ロジック

- [x] [RED] バージョンチェッカーのテスト作成 (`src/version/checker.rs`内の`#[cfg(test)]`)
  - `check_version()`でキャッシュから最新バージョンを取得し、現在のバージョンと比較
  - `CheckResult`を返すことを確認
  - テストケース: Latest, Outdated, Newer, NotInCache, NotFound, Invalid

- [x] [GREEN] バージョンチェッカーの実装 (`src/version/checker.rs`)
  - `check_version(cache, registry_type, package_name, current_version) -> Result<CheckResult>`
  - キャッシュからバージョン一覧を取得
  - `version_exists()`で存在チェック
  - `get_latest_version()`で最新バージョン取得
  - `compare_versions()`でバージョン比較

- [x] [REFACTOR] キャッシュのget_latest_versionにORDER BY追加
  - 挿入順序を保証するためにORDER BY v.id ASCを追加

---

### Phase 6: LSPサーバー骨格 ✅

#### 6.1 LSPサーバー起動

- [x] [GREEN] LSPサーバーの実装 (`src/lsp/server.rs`, `src/lsp/backend.rs`)
  - `main.rs`でサーバー起動
  - tower-lspの`LanguageServer`トレイト実装
  - `initialize()`でServerCapabilitiesを返す

- [x] [REFACTOR] ロギングとエラーハンドリング
  - JSON形式のファイルロギング (`src/log.rs`)
  - RUST_LOGによるログレベル設定

#### 6.2 キャッシュの初期化

- [x] [GREEN] キャッシュ初期化の実装
  - サーバー起動時に`Cache::new()`を呼び出し
  - `tokio::spawn()`でバックグラウンド更新タスクを起動
  - `get_packages_needing_refresh()`で古いパッケージを取得

- [x] [STRUCTURAL] 設定モジュールの追加 (`src/config.rs`)
  - XDG_DATA_HOME準拠のデータディレクトリ
  - データベースパス、ログパスの設定

---

### Phase 7: Diagnostics生成と公開 ✅

#### 7.1 Diagnostics生成

- [x] [RED] Diagnostics生成のテスト作成 (`src/lsp/diagnostics.rs`内の`#[cfg(test)]`)
  - `VersionCompareResult`から`Diagnostic`を生成できることを確認
  - 各ステータスに対応するseverityとメッセージを確認

- [x] [GREEN] Diagnostics生成の実装 (`src/lsp/diagnostics.rs`)
  - `create_diagnostic(package, result) -> Option<Diagnostic>`
  - `generate_diagnostics(uri, content, parser, resolver) -> Vec<Diagnostic>`
  - severityの決定（Latest→表示なし, Outdated→Warning, Newer/NotFound/Invalid→Error）

#### 7.2 ファイル監視とDiagnostics公開

- [x] [RED] ファイル監視のテスト作成
  - `generate_diagnostics`の振る舞いテスト
  - 各ステータスに対する診断結果の確認

- [x] [GREEN] ファイル監視の実装 (`src/lsp/backend.rs`)
  - `textDocument/didOpen`ハンドラー
  - ファイル内容のパースとバージョンチェック
  - `publish_diagnostics`でDiagnosticsを公開

---

### Phase 7.5: レジストリ-キャッシュ連携 ✅

#### 7.5.1 バックグラウンド更新の実装

- [x] [RED] バックグラウンド更新のテスト作成 (`src/lsp/refresh.rs`内の`#[cfg(test)]`)
  - 古いパッケージに対してレジストリAPIが呼ばれることを確認
  - 取得したバージョンがキャッシュに保存されることを確認

- [x] [GREEN] バックグラウンド更新の実装 (`src/lsp/refresh.rs`, `src/lsp/backend.rs`)
  - `spawn_background_refresh`内でレジストリAPIを呼び出し
  - `GitHubRegistry::fetch_all_versions()`で全バージョンを取得
  - `Cache::replace_versions()`でキャッシュに保存
  - エラー時はログ出力して継続（他のパッケージの更新は続行）

- [x] [REFACTOR] 並列処理の最適化 - YAGNIでスキップ
  - 現時点では逐次処理で十分

#### 7.5.2 オンデマンド取得の実装

- [x] [RED] オンデマンド取得のテスト作成
  - キャッシュミス時にレジストリAPIが呼ばれることを確認
  - 取得したバージョンがキャッシュに保存されることを確認

- [x] [GREEN] オンデマンド取得の実装 (`src/lsp/refresh.rs`, `src/lsp/backend.rs`)
  - `did_open`ハンドラー内でキャッシュミスを検出
  - レジストリAPIを呼び出してバージョンを取得
  - キャッシュに保存
  - Diagnosticsを再生成して公開

- [x] [REFACTOR] 非同期処理の改善
  - API呼び出し中もエディタがブロックしない（tokio::spawn使用）
  - 取得完了後に`publish_diagnostics`で更新通知

---

### Phase 8: 統合テスト ✅

#### 8.1 E2Eテスト

- [x] [RED] E2Eテストの作成 (`tests/e2e_test.rs`)
  - ワークフローファイルをパースしてDiagnosticsを生成
  - 各シナリオのテスト（最新、古い、存在しない、キャッシュなし、混在）

- [x] [GREEN] テストフィクスチャの準備
  - インラインでワークフローコンテンツを定義
  - `create_test_cache_with_versions`ヘルパー関数でモックキャッシュを作成

- [x] [REFACTOR] テストの安定化
  - semver形式のバージョン（4.0.0）を使用してパース結果と一致させる
  - tempfileでテストごとに独立したDBを使用

---

### Phase 9: package.jsonパーサー

#### 9.1 package.jsonパーサーの実装

- [x] [RED] package.jsonパーサーのテスト作成 (`src/parser/package_json.rs`内の`#[cfg(test)]`)
  - `dependencies`、`devDependencies`を抽出できることを確認
  - バージョン範囲（`^1.0.0`、`~1.0.0`等）のパース

- [x] [GREEN] package.jsonパーサーの実装 (`src/parser/package_json.rs`)
  - tree-sitter-jsonを使用
  - `dependencies`、`devDependencies`、`peerDependencies`の抽出

- [x] [REFACTOR] バージョン範囲の正規化
  - npm registry API統合時（Phase 10）に実装予定
  - パーサーはソースのバージョン文字列をそのまま抽出

---

### Phase 10: npm registry API

#### 10.1 npm registry APIの実装

- [x] [RED] npm registry APIのテスト作成 (`src/version/registries/npm.rs`内の`#[cfg(test)]`)
  - モックAPIサーバーでテスト
  - `fetch_all_versions()`で全バージョンを取得できることを確認

- [x] [GREEN] npm registry APIの実装 (`src/version/registries/npm.rs`)
  - reqwestでnpm registry (`https://registry.npmjs.org/{package}`)を呼び出し
  - `versions`フィールドから全バージョンを抽出

- [x] [REFACTOR] エラーハンドリング
  - 存在しないパッケージ（404）
  - scoped packages (`@types/node`)の対応

---

### Phase 10.5: VersionMatcher 抽象化（リファクタリング）

レジストリごとのバージョンマッチングロジックを抽象化し、npm等の新しいレジストリ追加を容易にする。
npm の範囲指定 (`^1.0.0`, `~1.0.0`) と GitHub Actions の部分マッチング (`v6` → `v6.x.x`) を統一的に扱えるようにする。

参照: `docs/DESIGN-version-comparison.md`

#### 10.5.1 VersionMatcher トレイト定義

- [x] [STRUCTURAL] VersionMatcher トレイトの定義 (`src/version/matcher.rs`)
  - `VersionMatcher` トレイト定義
  - `registry_type(&self) -> RegistryType`
  - `version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool`
  - `compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult`

#### 10.5.2 GitHubActionsMatcher 実装

- [x] [RED] GitHubActionsMatcher のテスト作成 (`src/version/matchers/github_actions.rs`内の`#[cfg(test)]`)
  - 部分バージョンマッチング (`v6` → `v6.x.x`) のテスト
  - バージョン比較のテスト（メジャーのみ、メジャー.マイナー、フルバージョン）

- [x] [GREEN] GitHubActionsMatcher の実装 (`src/version/matchers/github_actions.rs`)
  - `semver.rs` の既存ロジック (`version_matches_any`, `compare_versions`) を移行
  - `VersionMatcher` トレイトを実装

- [x] [REFACTOR] semver.rs の整理
  - GitHub Actions 固有のロジックを `GitHubActionsMatcher` に集約
  - `semver.rs` には共通ユーティリティ (`normalize_version`) のみ残す

#### 10.5.3 checker.rs 更新

- [x] [RED] compare_version の新シグネチャのテスト作成
  - `compare_version(storer, matcher, package_name, version)` 形式のテスト
  - 既存テストケースを新シグネチャに対応

- [x] [GREEN] compare_version の更新 (`src/version/checker.rs`)
  - `registry_type: &str` パラメータを `matcher: &dyn VersionMatcher` に変更
  - `matcher.version_exists()` と `matcher.compare_to_latest()` を使用

- [x] [REFACTOR] 既存テストの更新
  - MockStorer を使用したテストを新シグネチャに対応

#### 10.5.4 diagnostics.rs 更新

- [x] [RED] generate_diagnostics の新シグネチャのテスト作成
  - `generate_diagnostics(parser, matcher, storer, content)` 形式のテスト

- [x] [GREEN] generate_diagnostics の更新 (`src/lsp/diagnostics.rs`)
  - `matcher: &dyn VersionMatcher` パラメータを追加
  - `compare_version()` 呼び出しに matcher を渡す

- [x] [REFACTOR] 既存テストの更新
  - GitHubActionsMatcher を使用してテストを更新

#### 10.5.5 Backend 統合

- [x] [GREEN] Backend に matchers HashMap を追加 (`src/lsp/backend.rs`)
  - `matchers: HashMap<RegistryType, Arc<dyn VersionMatcher>>` フィールド追加
  - `initialize_matchers()` メソッド追加
  - `generate_diagnostics()` 呼び出しを更新

- [x] [REFACTOR] 動作確認
  - 既存の GitHub Actions ワークフローに対する動作確認
  - 全テスト通過確認

---

### Phase 10.6: NpmVersionMatcher 実装

#### 10.6.1 NpmVersionMatcher 実装

- [x] [RED] NpmVersionMatcher のテスト作成 (`src/version/matchers/npm.rs`内の`#[cfg(test)]`)
  - 範囲指定のテスト (`^1.0.0`, `~1.0.0`, `>=1.0.0`, etc.)
  - 完全一致のテスト (`1.0.0`)
  - 無効な範囲指定のテスト

- [x] [GREEN] NpmVersionMatcher の実装 (`src/version/matchers/npm.rs`)
  - 範囲指定のパース (`^`, `~`, `>=`, `>`, `<=`, `<`, `x`, `*`)
  - 範囲内で最新バージョンを見つける
  - `VersionMatcher` トレイトを実装

- [x] [REFACTOR] エッジケースの対応
  - プレリリースバージョンの処理
  - ワイルドカード (`x`, `*`) の処理

#### 10.6.2 package.json統合とE2Eテスト

- [x] [RED] E2Eテスト作成 (`tests/lsp_e2e_test.rs`)
  - package.jsonのdidOpen時にdiagnosticsが発行されることを確認
  - 古いバージョン、存在しないバージョンのケース

- [x] [GREEN] Backend統合
  - `initialize_parsers()`に`PackageJsonParser`を追加
  - `initialize_registries()`に`NpmRegistry`を追加
  - `initialize_matchers()`に`NpmVersionMatcher`を追加

- [x] [REFACTOR] 動作確認
  - package.json に対する動作確認
  - 全テスト通過確認

---

### Phase 11: Cargo.tomlパーサー

#### 11.1 Cargo.tomlパーサーの実装

- [ ] [RED] Cargo.tomlパーサーのテスト作成 (`src/parser/cargo_toml.rs`内の`#[cfg(test)]`)
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

- [ ] [RED] crates.io APIのテスト作成 (`src/version/registries/crates_io.rs`内の`#[cfg(test)]`)
  - モックAPIサーバーでテスト
  - `fetch_all_versions()`で全バージョンを取得できることを確認

- [ ] [GREEN] crates.io APIの実装 (`src/version/registries/crates_io.rs`)
  - reqwestでcrates.io API (`https://crates.io/api/v1/crates/{crate}`)を呼び出し
  - `versions`配列から全バージョンを抽出

- [ ] [REFACTOR] エラーハンドリング
  - 存在しないクレート（404）

#### 12.2 CratesVersionMatcher 実装

- [ ] [RED] CratesVersionMatcher のテスト作成 (`src/version/matchers/crates.rs`内の`#[cfg(test)]`)
  - Cargo.tomlのバージョン要件テスト (`^1.0`, `~1.0`, `>=1.0`, `=1.0`, etc.)
  - 完全一致のテスト (`1.0.0`)

- [ ] [GREEN] CratesVersionMatcher の実装 (`src/version/matchers/crates.rs`)
  - Cargoのバージョン要件パース（npmと類似だが微妙に異なる）
  - `VersionMatcher` トレイトを実装

- [ ] [REFACTOR] エッジケースの対応
  - ワイルドカード (`*`) の処理

#### 12.3 Cargo.toml統合とE2Eテスト

- [ ] [RED] E2Eテスト作成 (`tests/lsp_e2e_test.rs`)
  - Cargo.tomlのdidOpen時にdiagnosticsが発行されることを確認
  - 古いバージョン、存在しないバージョンのケース

- [ ] [GREEN] Backend統合
  - `initialize_parsers()`に`CargoTomlParser`を追加
  - `initialize_registries()`に`CratesRegistry`を追加
  - `initialize_matchers()`に`CratesVersionMatcher`を追加

- [ ] [REFACTOR] 動作確認
  - Cargo.toml に対する動作確認
  - 全テスト通過確認

---

### Phase 13: go.modパーサー

#### 13.1 go.modパーサーの実装

- [ ] [RED] go.modパーサーのテスト作成 (`src/parser/go_mod.rs`内の`#[cfg(test)]`)
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

- [ ] [RED] Go proxy APIのテスト作成 (`src/version/registries/go_proxy.rs`内の`#[cfg(test)]`)
  - モックAPIサーバーでテスト
  - `fetch_all_versions()`で全バージョンを取得できることを確認

- [ ] [GREEN] Go proxy APIの実装 (`src/version/registries/go_proxy.rs`)
  - reqwestでGo proxy (`https://proxy.golang.org/{module}/@v/list`)を呼び出し
  - バージョンリストを取得

- [ ] [REFACTOR] エラーハンドリング
  - 存在しないモジュール（404/410）

#### 14.2 GoVersionMatcher 実装

- [ ] [RED] GoVersionMatcher のテスト作成 (`src/version/matchers/go.rs`内の`#[cfg(test)]`)
  - Goのsemverバージョンテスト (`v1.0.0`, `v1.0.0+incompatible`)
  - pseudo-versionsのテスト

- [ ] [GREEN] GoVersionMatcher の実装 (`src/version/matchers/go.rs`)
  - Goのバージョン形式パース
  - `VersionMatcher` トレイトを実装

- [ ] [REFACTOR] エッジケースの対応
  - pseudo-versionsの比較

#### 14.3 go.mod統合とE2Eテスト

- [ ] [RED] E2Eテスト作成 (`tests/lsp_e2e_test.rs`)
  - go.modのdidOpen時にdiagnosticsが発行されることを確認
  - 古いバージョン、存在しないバージョンのケース

- [ ] [GREEN] Backend統合
  - `initialize_parsers()`に`GoModParser`を追加
  - `initialize_registries()`に`GoProxyRegistry`を追加
  - `initialize_matchers()`に`GoVersionMatcher`を追加

- [ ] [REFACTOR] 動作確認
  - go.mod に対する動作確認
  - 全テスト通過確認

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
