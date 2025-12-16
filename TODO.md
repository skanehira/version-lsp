# 改善タスクリスト

## Critical

- [x] 1. Lock poisoning `.expect()` をエラーハンドリングに変更
  - 場所: `src/version/cache.rs` (11箇所)
  - 問題: RwLock が poisoned になった場合に panic する
  - 対応: `CacheError::LockPoisoned` を追加し、`lock_conn()` ヘルパーメソッドで適切にエラーハンドリング

## High

- [x] 2. fetch処理のコード重複を解消
  - 場所: `src/lsp/refresh.rs`
  - 問題: `refresh_packages()` と `fetch_missing_packages()` で約70%重複
  - 対応: `fetch_and_cache_package()` ヘルパー関数を抽出（約90行削減）

- [x] 3. 時刻計算の重複を解消
  - 場所: `src/version/cache.rs` (4箇所)
  - 問題: `SystemTime::now().duration_since(UNIX_EPOCH)` が重複
  - 対応: `current_timestamp_ms()` ヘルパーメソッドを追加

## Medium

- [x] 4. バージョン保存の差分更新
  - 場所: `src/version/cache.rs`
  - 問題: `replace_versions()` が全削除→全挿入で非効率（2000バージョン以上のパッケージで問題）
  - 対応: `INSERT OR IGNORE` で新しいバージョンのみ追加する方式に変更

- [ ] 5. パース失敗時のログ出力
  - 場所: `src/lsp/backend.rs:214, 260`
  - 問題: `.unwrap_or_default()` でエラーが隠される

## Low

- [ ] 6. マジック定数の集約
  - 問題: 定数が複数ファイルに分散している

- [ ] 7. モジュールレベルのドキュメント追加
  - 場所: `src/version/` モジュール
