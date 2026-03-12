# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Build & Run
```bash
cargo build
cargo build --release

# Run as LSP server (communicates via stdin/stdout)
cargo run
```

### Testing
```bash
# Run all tests
cargo test

# Fast test execution with cargo-nextest (recommended)
cargo nextest run

# Run a single test
cargo test test_name

# Run E2E tests for a specific registry
cargo test --test e2e_npm
cargo test --test e2e_crates
cargo test --test e2e_docker

# Generate coverage (requires cargo-llvm-cov)
cargo llvm-cov nextest --lcov --output-path lcov.info
```

### Quality Checks
```bash
cargo fmt -- --check
cargo fmt
cargo clippy
```

## Architecture

version-lsp is a Language Server Protocol implementation that checks package dependency versions across multiple registries (npm, crates.io, Go proxy, PyPI, GitHub Actions, JSR, pnpm catalogs, Docker Hub/ghcr.io).

詳細なアーキテクチャ（データフロー図、DB スキーマ、各コンポーネントの責務など）は [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) を参照すること。実装・設計変更時は必ずこのドキュメントを確認し、整合性を保つこと。

### Core Data Flow

```
File opened/changed in editor
  → Parser (tree-sitter) extracts PackageInfo[]
  → Cache (SQLite) lookup for stored versions
  → VersionMatcher compares current vs latest
  → Diagnostics published to editor
  → Background task: Registry fetch for missing/stale packages → re-publish diagnostics
```

### Three-Layer Architecture

**Parser layer** (`src/parser/`): Each supported file format has a parser implementing the `Parser` trait. Uses tree-sitter for structured parsing (YAML, JSON, TOML). `detect_parser_type()` in `types.rs` routes URIs to the correct parser based on filename.

**Version layer** (`src/version/`): Three coordinated components:
- `Registry` trait + `registries/`: Async HTTP fetchers for each package registry
- `Cache` (`cache.rs`): SQLite storage with WAL mode, fetch deduplication, and refresh intervals
- `VersionMatcher` trait + `matchers/`: Registry-specific comparison logic (semver ranges for npm/crates, partial version matching for GitHub Actions, PEP 508 for PyPI, suffix-aware Docker tag comparison)

**LSP layer** (`src/lsp/`):
- `Backend` is generic over `VersionStorer` trait (enables test injection)
- `PackageResolver` (`resolver.rs`) groups Parser + VersionMatcher + Registry per registry type
- `diagnostics.rs` converts `VersionCompareResult` into LSP diagnostics
- `code_action.rs` generates version bump code actions
- `refresh.rs` handles background package fetching

### Key Traits

| Trait | Location | Purpose |
|-------|----------|---------|
| `Parser` | `parser/traits.rs` | Parse files into `PackageInfo[]` |
| `Registry` | `version/registry.rs` | Fetch versions from remote registries |
| `VersionMatcher` | `version/matcher.rs` | Registry-specific version comparison |
| `VersionStorer` | `version/checker.rs` | Cache read/write abstraction |

All four traits use `#[cfg_attr(test, automock)]` for mock generation with `mockall`.

### Testing Patterns

- **Unit tests**: In-file `#[cfg(test)] mod tests` with `mockall` mocks and `rstest` for parameterized tests
- **E2E tests** (`tests/`): Each registry has its own E2E file. Uses `tests/helper/` for LSP request builders, mock registries, and real `Cache` instances with `tempfile`
- E2E tests create `LspService` with `Backend::build()` to inject test cache and mock registries

### Key Settings
- Rust toolchain: 1.92.0 (`rust-toolchain.toml`)
- Edition: 2024
- Test dependencies: `mockall`, `mockito`, `rstest`, `serial_test`, `tempfile`
