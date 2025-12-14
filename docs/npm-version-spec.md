# npm package.json バージョン記述仕様

npm package.json の dependencies に記述できるバージョン指定形式の調査結果。

## 参考資料

- [npm-package-arg - GitHub](https://github.com/npm/npm-package-arg)
- [npm install - npm Docs](https://docs.npmjs.com/cli/v6/commands/npm-install/)
- [Package Aliases RFC - npm/rfcs](https://github.com/npm/rfcs/blob/main/implemented/0001-package-aliases.md)
- [pnpm Workspaces](https://pnpm.io/workspaces)
- [pnpm Aliases](https://pnpm.io/aliases)
- [SchemaStore package.json schema](https://raw.githubusercontent.com/SchemaStore/schemastore/refs/heads/master/src/schemas/json/package.json)

---

## 1. Semver 範囲指定

| 形式 | 例 | 説明 |
|------|----|----|
| Exact | `"1.2.3"` | 完全一致 |
| Caret | `"^1.2.3"` | メジャー互換 (1.x.x) |
| Tilde | `"~1.2.3"` | マイナー互換 (1.2.x) |
| Greater/Less | `">=1.0.0"`, `"<2.0.0"` | 比較演算子 |
| Range | `">=1.0.0 <2.0.0"` | AND 範囲（スペース区切り） |
| Hyphen | `"1.0.0 - 2.0.0"` | 範囲（両端含む） |
| OR | `"^1.0.0 \|\| ^2.0.0"` | OR 結合 |
| Wildcard | `"*"`, `"1.x"`, `"1.2.x"` | ワイルドカード |

### 例

```json
{
  "dependencies": {
    "exact": "1.2.3",
    "caret": "^1.2.3",
    "tilde": "~1.2.3",
    "gte": ">=1.0.0",
    "range": ">=1.0.0 <2.0.0",
    "hyphen": "1.0.0 - 2.0.0",
    "or": "^1.0.0 || ^2.0.0",
    "wildcard": "*",
    "partial": "1.x"
  }
}
```

---

## 2. Dist Tags

レジストリで定義されたタグを参照する形式。

```json
{
  "dependencies": {
    "react": "latest",
    "next": "canary",
    "typescript": "beta",
    "webpack": "next"
  }
}
```

### 一般的なタグ

| タグ | 説明 |
|------|------|
| `latest` | 最新の安定版（デフォルト） |
| `next` | 次期メジャーバージョンのプレビュー |
| `beta` | ベータ版 |
| `alpha` | アルファ版 |
| `canary` | 最新の開発版 |
| `rc` | リリース候補 |

---

## 3. npm Alias (`npm:` プロトコル)

パッケージを別名でインストールする形式。

### 構文

```
<alias>@npm:<package>@<version>
```

### 例

```json
{
  "dependencies": {
    "vite": "npm:rolldown-vite@7.2.2",
    "jquery2": "npm:jquery@2",
    "jquery3": "npm:jquery@3",
    "my-lodash": "npm:lodash@^4.0.0",
    "npa": "npm:npm-package-arg"
  }
}
```

### 用途

- 同一パッケージの複数バージョンを並行利用
- フォークを元のパッケージ名でインストール
- 長いパッケージ名に短いエイリアスを付与

### 制限事項

- レジストリパッケージのみ対応（Git URL などには使用不可）
- トランジティブ依存関係には影響しない

---

## 4. Git URL

Git リポジトリから直接インストールする形式。

### 構文

```
<protocol>://[<user>[:<password>]@]<hostname>[:<port>][:][/]<path>[#<commit-ish> | #semver:<semver>]
```

### 対応プロトコル

- `git`
- `git+ssh`
- `git+https`
- `git+http`
- `git+file`

### 例

```json
{
  "dependencies": {
    "pkg1": "git+ssh://git@github.com:user/repo.git",
    "pkg2": "git+https://github.com/user/repo.git",
    "pkg3": "git+https://github.com/user/repo.git#v1.0.0",
    "pkg4": "git+https://github.com/user/repo.git#develop",
    "pkg5": "git+https://github.com/user/repo.git#semver:^1.0.0",
    "pkg6": "git+https://github.com/user/repo.git#abc1234"
  }
}
```

### コミット指定

| 形式 | 説明 |
|------|------|
| `#v1.0.0` | タグ |
| `#develop` | ブランチ |
| `#abc1234` | コミットハッシュ |
| `#semver:^1.0.0` | semver 範囲でタグを検索 |
| (なし) | デフォルトブランチ |

---

## 5. GitHub/GitLab/Bitbucket ショートカット

ホスティングサービスの短縮形式。

### 構文

```
<provider>:<user>/<repo>[#<commit-ish>]
```

### 例

```json
{
  "dependencies": {
    "express": "github:expressjs/express",
    "lodash": "github:lodash/lodash#4.17.21",
    "foo": "gitlab:user/repo",
    "bar": "bitbucket:user/repo",
    "baz": "gist:hash"
  }
}
```

### GitHub 省略形

GitHub の場合は `github:` を省略可能:

```json
{
  "dependencies": {
    "express": "expressjs/express"
  }
}
```

---

## 6. HTTP/HTTPS URL (Tarball)

tarball を直接 URL から取得する形式。

### 例

```json
{
  "dependencies": {
    "pkg": "https://example.com/package.tgz",
    "other": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz"
  }
}
```

### 要件

- URL は `http://` または `https://` で始まる必要がある
- 通常は `.tgz` または `.tar.gz` ファイルを指す

---

## 7. ローカルパス (`file:` プロトコル)

ローカルファイルシステムのパッケージを参照する形式。

### 例

```json
{
  "dependencies": {
    "local-pkg": "file:../my-package",
    "tarball": "file:./packages/foo.tgz",
    "relative": "../sibling-package",
    "absolute": "file:/absolute/path/to/package"
  }
}
```

### 対応形式

- ディレクトリパス（`package.json` を含むフォルダ）
- tarball ファイル（`.tgz`, `.tar.gz`, `.tar`）
- `file:` プレフィックス付きまたは相対パス直接指定

### 注意事項

- `file:` URL は URI エンコードされない
- `/` または `./` で始まるパスを推奨

---

## 8. Workspace プロトコル

モノレポ内のローカルパッケージを参照する形式。主に pnpm, Yarn, Bun で使用。

### 構文

```
workspace:<version-or-alias>
```

### 例

```json
{
  "dependencies": {
    "shared-utils": "workspace:*",
    "ui-components": "workspace:^1.0.0",
    "alias-pkg": "workspace:actual-name@*",
    "exact": "workspace:1.0.0"
  }
}
```

### 動作

- `workspace:*` - ワークスペース内のパッケージにシンボリックリンク
- publish 時に実際のバージョンに置換される
- npm v7+ のワークスペースでは自動解決（`workspace:` プレフィックス不要）

---

## 9. その他のプロトコル (pnpm 固有)

pnpm が追加でサポートするプロトコル。

| プロトコル | 例 | 説明 |
|------------|----|----|
| `link:` | `"link:../foo"` | シンボリックリンク（node_modules 内に作成） |
| `portal:` | `"portal:../foo"` | ポータルリンク（依存関係も含めてリンク） |
| `catalog:` | `"catalog:react"` | カタログから参照（pnpm v9+） |

---

## npm-package-arg の Type 分類

[npm-package-arg](https://github.com/npm/npm-package-arg) ライブラリが返す `type` 値:

| type | 説明 | 例 |
|------|------|----|
| `version` | 完全バージョン | `foo@1.2.3` |
| `range` | semver 範囲 | `foo@^1.0.0`, `foo@~1.0.0` |
| `tag` | dist-tag | `foo@latest`, `foo@beta` |
| `alias` | npm エイリアス | `myalias@npm:foo@1.2.3` |
| `git` | Git リポジトリ | `git+https://github.com/user/repo` |
| `remote` | HTTP URL (tarball) | `https://example.com/pkg.tgz` |
| `file` | ローカル tarball | `file:./foo.tgz` |
| `directory` | ローカルディレクトリ | `file:../foo`, `../foo` |

---

## version-lsp での対応状況

### 現在サポート済み

- [x] Exact version (`1.2.3`)
- [x] Caret (`^1.2.3`)
- [x] Tilde (`~1.2.3`)
- [x] Comparison operators (`>=`, `<=`, `>`, `<`)
- [x] Wildcards (`*`, `x`, `1.x`, `1.2.x`)
- [x] Hyphen ranges (`1.0.0 - 2.0.0`)
- [x] OR ranges (`^1.0.0 || ^2.0.0`)
- [x] AND ranges (`>=1.0.0 <2.0.0`)
- [x] Dist tags (`latest`, `beta`, `canary`)
- [x] npm alias (`npm:package@version`)

### 未サポート（対応予定なし）

- [ ] Git URLs (`git+https://...`) - バージョンチェック不要
- [ ] GitHub shortcuts (`github:user/repo`) - バージョンチェック不要
- [ ] HTTP URLs (`https://...tgz`) - バージョンチェック不要
- [ ] File paths (`file:...`) - バージョンチェック不要
- [ ] Workspace protocol (`workspace:*`) - ローカル参照のため
