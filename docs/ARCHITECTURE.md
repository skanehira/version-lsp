# version-lsp Architecture Document

## Overview

version-lsp is a Language Server Protocol (LSP) implementation that provides version checking diagnostics for package dependency files (package.json, Cargo.toml, go.mod, GitHub Actions workflow).

**Key Features:**
- Detection and warning for outdated versions
- Error display for non-existent versions
- Error display for invalid version formats
- Support for version range specifications in each ecosystem

**Supported Registries:**
| Registry        | File Format         | Version Specification                        |
|-----------------|---------------------|----------------------------------------------|
| npm             | package.json        | semver range (`^`, `~`, `>=`, `||`, etc.)    |
| crates.io       | Cargo.toml          | Cargo requirements (`^`, `~`, `=`, `*`, etc.)|
| Go Proxy        | go.mod              | Exact match                                  |
| GitHub Releases | GitHub Actions YAML | Partial match (`v4` → `v4.x.x`)              |

---

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        LSP Protocol Layer                           │
│                         (tower-lsp)                                 │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                           Backend                                   │
│                    (LSP Server Core)                                │
│  ┌──────────────┐ ┌───────────────┐ ┌──────────────┐                │
│  │ Config       │ │ Resolvers     │ │ Cache        │                │
│  │ Manager      │ │ (per registry)│ │ (SQLite)     │                │
│  └──────────────┘ └───────────────┘ └──────────────┘                │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                   Package Resolution Pipeline                       │
├─────────────────────┬─────────────────────┬─────────────────────────┤
│    Parser Layer     │   Matcher Layer     │    Registry Layer       │
│    ─────────────    │   ─────────────     │    ──────────────       │
│  • PackageJson      │  • NpmMatcher       │  • NpmRegistry          │
│  • CargoToml        │  • CratesMatcher    │  • CratesRegistry       │
│  • GoMod            │  • GoMatcher        │  • GoProxyRegistry      │
│  • GitHubActions    │  • GitHubMatcher    │  • GitHubRegistry       │
└─────────────────────┴─────────────────────┴─────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    Version Management Layer                         │
├─────────────────────┬─────────────────────┬─────────────────────────┤
│   Cache (SQLite)    │      Checker        │     Semver Utils        │
│   ──────────────    │      ───────        │     ────────────        │
│  • Version Storage  │  • Version Compare  │  • Version Parser       │
│  • Dist Tags        │  • Status Enum      │  • Normalization        │
│  • Fetch Locking    │  • Dist Tag Resolve │                         │
│  • Refresh Logic    │                     │                         │
└─────────────────────┴─────────────────────┴─────────────────────────┘
```

---

## Module Structure

```
src/
├── main.rs                  # Entry point (Tokio async runtime initialization)
├── lib.rs                   # Library root
├── config.rs                # Configuration management & file paths
├── log.rs                   # Log initialization
│
├── lsp/                     # LSP Server Implementation
│   ├── mod.rs              # Module documentation
│   ├── server.rs           # LSP server startup & lifecycle
│   ├── backend.rs          # LanguageServer trait implementation
│   ├── diagnostics.rs      # Diagnostic generation logic
│   ├── resolver.rs         # PackageResolver (parser/matcher/registry integration)
│   └── refresh.rs          # Background refresh & on-demand fetch logic
│
├── parser/                  # File Parsing Layer
│   ├── mod.rs              # Module exports
│   ├── traits.rs           # Parser trait definition
│   ├── types.rs            # RegistryType, PackageInfo, parser detection
│   ├── package_json.rs     # npm package.json parser
│   ├── cargo_toml.rs       # Rust Cargo.toml parser
│   ├── github_actions.rs   # GitHub Actions workflow parser
│   └── go_mod.rs           # Go go.mod parser
│
└── version/                 # Version Management Layer
    ├── mod.rs              # Module documentation & architecture diagram
    ├── types.rs            # PackageVersions struct
    ├── error.rs            # CacheError, RegistryError enums
    ├── registry.rs         # Registry trait definition
    ├── matcher.rs          # VersionMatcher trait definition
    ├── checker.rs          # Version comparison & VersionStorer trait
    ├── semver.rs           # Semver utilities
    ├── cache.rs            # Cache implementation (SQLite)
    │
    ├── registries/         # Registry Implementations
    │   ├── mod.rs
    │   ├── npm.rs          # npm registry API client
    │   ├── crates_io.rs    # crates.io API client
    │   ├── github.rs       # GitHub Releases API client
    │   └── go_proxy.rs     # Go Proxy API client
    │
    └── matchers/           # Version Matcher Implementations
        ├── mod.rs
        ├── npm.rs          # npm semver range matching
        ├── crates.rs       # Cargo version requirements
        ├── github_actions.rs # GitHub Actions partial version matching
        └── go.rs           # Go exact matching
```

---

## Data Flow

### 1. Document Open Flow

```
User opens package.json
           │
           ▼
LSP Client sends textDocument/didOpen
           │
           ▼
Backend::did_open() receives notification
           │
           ▼
Detect registry type from URI
(package.json → Npm, Cargo.toml → CratesIo, etc.)
           │
           ▼
Get appropriate PackageResolver
           │
           ▼
Parser.parse(content) → Vec<PackageInfo>
           │
           ▼
┌──────────────────────────────────────────┐
│         generate_diagnostics()           │
│                                          │
│  For each PackageInfo:                   │
│    1. Call compare_version()             │
│       - Get latest version from cache    │
│       - Resolve dist-tag (for npm)       │
│       - Check version existence          │
│       - Compare current vs latest        │
│    2. Create diagnostic based on status  │
│       - Latest, Newer → skip             │
│       - NotInCache → skip                │
│       - Outdated → WARNING               │
│       - NotFound, Invalid → ERROR        │
└──────────────────────────────────────────┘
           │
           ▼
client.publish_diagnostics() publishes diagnostics
           │
           ▼
Spawn background task: fetch_missing_packages()
           │
           ▼
Fetch packages not in cache
           │
           ▼
Re-publish diagnostics after successful fetch
```

### 2. Background Refresh Flow

```
Backend::initialized() called
           │
           ▼
Spawn spawn_background_refresh() async task
           │
           ▼
cache.get_packages_needing_refresh()
(Get packages with updated_at older than refresh_interval)
           │
           ▼
Group by registry type
           │
           ▼
┌──────────────────────────────────────────┐
│   For each package (staggered 10ms):     │
│     1. try_start_fetch() to acquire lock │
│     2. registry.fetch_all_versions()     │
│     3. Save versions + dist_tags to cache│
│     4. finish_fetch() to release lock    │
│                                          │
│   ※ Continue processing even on errors   │
└──────────────────────────────────────────┘
```

### 3. Configuration Update Flow

```
Backend::initialized() called
           │
           ▼
Spawn spawn_fetch_configuration() async task
           │
           ▼
Send workspace/configuration request to LSP client
           │
           ▼
Client returns version-lsp configuration
           │
           ▼
Parse JSON into LspConfig struct
           │
           ▼
Update config RwLock
           │
           ▼
Configuration takes effect on subsequent document edits
```

---

## Key Component Details

### Backend (src/lsp/backend.rs)

Core implementation of the LSP server. Implements the `LanguageServer` trait.

**State:**
```rust
pub struct Backend {
    client: Client,                              // Bidirectional communication with LSP client
    storer: Option<Arc<Cache>>,                  // Version cache
    config: Arc<RwLock<LspConfig>>,              // Dynamic configuration
    resolvers: HashMap<RegistryType, PackageResolver>, // Resolver per registry
}
```

**Server Capabilities:**
- Text document synchronization: FULL mode (entire document sent on each change)
- Document open/close detection
- Hover, Completion, Goto Definition: not supported

### PackageResolver (src/lsp/resolver.rs)

Groups three components together:
- **Parser**: File format-specific parsing
- **VersionMatcher**: Registry-specific version comparison logic
- **Registry**: Network fetch operations

```rust
pub struct PackageResolver {
    parser: Box<dyn Parser>,
    matcher: Box<dyn VersionMatcher>,
    registry: Box<dyn Registry>,
}
```

### Cache (src/version/cache.rs)

SQLite-based version cache.

**Database Schema:**
```sql
packages:
  id INTEGER PRIMARY KEY
  registry_type TEXT        -- "npm", "crates_io", etc.
  package_name TEXT
  updated_at INTEGER        -- Millisecond timestamp
  fetching_since INTEGER    -- For fetch locking (NULL = not fetching)
  UNIQUE(registry_type, package_name)

versions:
  id INTEGER PRIMARY KEY
  package_id INTEGER        -- FK to packages
  version TEXT
  UNIQUE(package_id, version)

dist_tags:
  id INTEGER PRIMARY KEY
  package_id INTEGER        -- FK to packages
  tag_name TEXT             -- "latest", "beta", etc.
  version TEXT              -- "4.17.21"
  UNIQUE(package_id, tag_name)
```

**Features:**
- WAL mode for concurrent read support
- Thread-safe via `Mutex<Connection>`
- Fetch locking to prevent duplicate fetches
- `INSERT OR IGNORE` for incremental updates

### VersionMatcher (src/version/matcher.rs)

Trait for version comparison.

```rust
pub trait VersionMatcher: Send + Sync {
    fn registry_type(&self) -> RegistryType;
    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool;
    fn compare_to_latest(&self, current: &str, latest: &str) -> CompareResult;
}
```

**Implementations:**
| Matcher | Version Specification Example | Behavior |
|---------|------------------------------|----------|
| NpmMatcher | `^1.2.3`, `>=1.0.0 <2.0.0` | semver range evaluation |
| CratesMatcher | `1.2.3`, `~1.2`, `>=1, <2` | Cargo requirements |
| GoMatcher | `v1.2.3` | Exact match |
| GitHubMatcher | `v4`, `v4.1` | Partial match (major/minor) |

### Registry (src/version/registry.rs)

Trait for fetching versions from registries.

```rust
#[async_trait]
pub trait Registry: Send + Sync {
    fn registry_type(&self) -> RegistryType;
    async fn fetch_all_versions(&self, package_name: &str) -> Result<PackageVersions, RegistryError>;
}
```

**Implementations:**
| Registry | Endpoint | Notes |
|----------|----------|-------|
| NpmRegistry | `registry.npmjs.org/{pkg}` | dist-tags support, sorted by publish date |
| CratesRegistry | `crates.io/api/v1/crates/{pkg}` | Excludes yanked versions |
| GoProxyRegistry | `proxy.golang.org/{mod}/@v/list` | Module path encoding |
| GitHubRegistry | `api.github.com/repos/{owner/repo}/releases` | Rate limit handling |

---

## Configuration

### File Paths

| Item | Path |
|------|------|
| Cache DB | `$XDG_DATA_HOME/version-lsp/versions.db` or `~/.local/share/version-lsp/versions.db` |
| Log File | `$XDG_DATA_HOME/version-lsp/version-lsp.log` or `~/.local/share/version-lsp/version-lsp.log` |

### Configuration Structure

```json
{
  "version-lsp": {
    "cache": {
      "refreshInterval": 86400000
    },
    "registries": {
      "npm": { "enabled": true },
      "crates": { "enabled": true },
      "goProxy": { "enabled": true },
      "github": { "enabled": true }
    },
    "ignorePrerelease": true
  }
}
```

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `DEFAULT_REFRESH_INTERVAL_MS` | 86,400,000 (24 hours) | Cache refresh interval |
| `FETCH_TIMEOUT_MS` | 30,000 (30 seconds) | Fetch lock timeout |
| `FETCH_STAGGER_DELAY_MS` | 10 | Delay between fetch starts (rate limit mitigation) |

---

## External Dependencies

### Core LSP
- **tower-lsp**: LSP protocol implementation
- **async-trait**: Async trait support

### Parsing
- **tree-sitter**: Language parsing framework
- **tree-sitter-yaml/json/toml-ng**: Language grammars
- **regex**: go.mod parsing

### Version Management
- **semver**: Semantic version parsing
- **chrono**: Timestamp handling

### HTTP/Network
- **reqwest** (rustls-tls): Async HTTP client
- **tokio**: Async runtime

### Storage
- **rusqlite** (bundled): SQLite database
- **indexmap**: Ordered HashMap (for npm version order preservation)

### Serialization
- **serde/serde_json**: JSON serialization

### Error Handling & Logging
- **thiserror**: Error type derivation
- **tracing**: Structured logging

---

## Test Structure

```
src/
├── **/mod.rs          # Each module has #[cfg(test)] mod tests
│
tests/
└── lsp_e2e_test.rs    # E2E LSP protocol tests
```

### Test Patterns

| Type | Location | Purpose |
|------|----------|---------|
| Unit Tests | Within implementation files | Parser correctness, matcher logic, cache operations |
| Integration Tests | tests/ | Component interactions |
| E2E Tests | tests/lsp_e2e_test.rs | Complete LSP protocol flows |

### Test Tools
- **mockall**: Trait mocking
- **mockito**: HTTP mocking
- **rstest**: Parameterized tests
- **tempfile**: Temporary files
