# TODO: JSR (deno.json) Support

作成日: 2025-12-22
生成元: task-planning
設計書: docs/DESIGN.md

## 概要

version-lspにJSR (JavaScript Registry) 対応を追加する。
deno.jsonの`imports`フィールドからJSRパッケージを検出し、バージョン診断を提供する。

## 実装タスク

### フェーズ1: RegistryType拡張 (構造変更)

- [ ] [STRUCTURAL] `RegistryType::Jsr` バリアントを追加
  - `src/parser/types.rs` に `Jsr` を追加
  - `as_str()` に `"jsr"` を追加
  - `FromStr` に `"jsr"` パターンを追加
  - `detect_parser_type()` に `/deno.json` パターンを追加
  - 既存テストが通ることを確認

### フェーズ2: DenoJsonParser実装 (動作変更)

- [ ] [RED] deno.json基本パースのテストを作成
  - `imports`フィールドからJSRパッケージを抽出するテスト
  - 入力: `{"imports": {"@luca/flag": "jsr:@luca/flag@^1.0.1"}}`
  - 期待: `PackageInfo { name: "@luca/flag", version: "^1.0.1", registry_type: Jsr, ... }`

- [ ] [GREEN] `DenoJsonParser` の最小実装
  - `src/parser/deno_json.rs` を作成
  - tree-sitter JSONパーサーを使用
  - `imports`フィールドを走査
  - `jsr:` プレフィックスをパース

- [ ] [RED] 複数パッケージのテストを作成
  - 複数のJSRパッケージを含むdeno.json
  - 順序が保持されることを確認

- [ ] [GREEN] 複数パッケージ対応

- [ ] [RED] エッジケースのテストを作成
  - バージョンなし: `"jsr:@std/path"` → version: "latest"
  - 非JSRエントリ: `"https://..."` → スキップ
  - 空の`imports`フィールド

- [ ] [GREEN] エッジケース対応

- [ ] [RED] オフセット計算のテストを作成
  - `start_offset`, `end_offset`, `line`, `column` が正確であること

- [ ] [GREEN] オフセット計算の実装

- [ ] [REFACTOR] パーサーコードの整理
  - `parse_jsr_specifier` ヘルパー関数の抽出
  - package_json.rsとの共通部分の検討

### フェーズ3: JsrRegistry実装 (動作変更)

- [ ] [RED] バージョン取得の基本テストを作成 (mockito)
  - モックサーバーからJSR APIレスポンスを返す
  - バージョンリストが正しく抽出されること

- [ ] [GREEN] `JsrRegistry` の最小実装
  - `src/version/registries/jsr.rs` を作成
  - `fetch_all_versions` を実装
  - `Accept: application/json` ヘッダーを設定

- [ ] [RED] バージョンソートのテストを作成
  - `createdAt`で古い順にソートされること
  - publish順とsemver順が異なるケース

- [ ] [GREEN] createdAtによるソート実装

- [ ] [RED] yankedバージョン除外のテストを作成
  - `yanked: true` のバージョンが結果に含まれないこと

- [ ] [GREEN] yankedフィルタリング実装

- [ ] [RED] エラーハンドリングのテストを作成
  - 404 Not Found → `RegistryError::NotFound`
  - 不正なJSON → `RegistryError::InvalidResponse`

- [ ] [GREEN] エラーハンドリング実装

- [ ] [REFACTOR] レジストリコードの整理

### フェーズ4: JsrVersionMatcher実装 (動作変更)

- [ ] [RED] バージョンマッチングのテストを作成
  - caret: `^1.0.0` が `1.0.0`, `1.9.9` にマッチ
  - tilde: `~1.2.0` が `1.2.0`, `1.2.9` にマッチ
  - exact: `1.0.0` が `1.0.0` のみにマッチ

- [ ] [GREEN] `JsrVersionMatcher` の実装
  - `src/version/matchers/jsr.rs` を作成
  - `npm_version_exists` に委譲
  - `npm_compare_to_latest` に委譲

- [ ] [RED] compare_to_latestのテストを作成
  - Latest, Outdated, Newer, Invalid の各ケース

- [ ] [GREEN] compare_to_latest実装確認

- [ ] [REFACTOR] npm.rsから共通関数をpub(crate)でエクスポート確認

### フェーズ5: 統合 (動作変更)

- [ ] [RED] リゾルバー登録のテストを作成
  - `create_default_resolvers` が `RegistryType::Jsr` を含むこと

- [ ] [GREEN] リゾルバーファクトリーに登録
  - `src/lsp/resolver.rs` を更新
  - `DenoJsonParser`, `JsrVersionMatcher`, `JsrRegistry` を組み合わせ

- [ ] [RED] モジュールエクスポートのテストを作成
  - `src/parser/mod.rs` から `DenoJsonParser` がエクスポートされること
  - `src/version/registries/mod.rs` から `JsrRegistry` がエクスポートされること
  - `src/version/matchers/mod.rs` から `JsrVersionMatcher` がエクスポートされること

- [ ] [GREEN] モジュール構成の更新

### フェーズ6: 品質保証

- [ ] 全テスト実行 (`cargo nextest run`)
- [ ] Clippy警告の解消 (`cargo clippy`)
- [ ] フォーマット確認 (`cargo fmt --check`)
- [ ] ビルド確認 (`cargo build --release`)

## 実装ノート

### MUSTルール遵守事項

- **TDD**: RED → GREEN → REFACTOR サイクルを厳守
- **Tidy First**: 構造変更(フェーズ1)と動作変更(フェーズ2-5)を分離
- **コミット**: 各タスク完了時にコミット
  - `[STRUCTURAL]` または `[BEHAVIORAL]` プレフィックス必須
  - テスト通過後のみコミット可能

### ファイル構成

```
src/
├── parser/
│   ├── mod.rs          # DenoJsonParser をエクスポート
│   ├── types.rs        # RegistryType::Jsr 追加
│   └── deno_json.rs    # 新規作成
└── version/
    ├── registries/
    │   ├── mod.rs      # JsrRegistry をエクスポート
    │   └── jsr.rs      # 新規作成
    └── matchers/
        ├── mod.rs      # JsrVersionMatcher をエクスポート
        └── jsr.rs      # 新規作成
```

### 参照ドキュメント

- 設計書: docs/DESIGN.md (JSR Support セクション)
- 既存実装参考: `src/parser/package_json.rs`, `src/version/registries/npm.rs`
