# version-lsp Design Document

## Design Principles

### 1. Separation of Concerns

Each layer has a single responsibility:
- **Parser Layer**: Understands only file formats
- **Matcher Layer**: Understands only version comparison rules
- **Registry Layer**: Understands only network protocols
- **Cache Layer**: Understands only storage
- **Checker Layer**: Orchestrates the above

### 2. Trait-Driven Design

Major components are defined as traits:
- `Parser`: File parsing
- `Registry`: Version fetching
- `VersionMatcher`: Version comparison
- `VersionStorer`: Cache operations

**Benefits:**
- Testability through mocking
- New implementations can be added without changing existing code
- Enables dependency injection pattern

### 3. Async-First

Fully asynchronous architecture based on Tokio:
- Non-blocking I/O operations
- Parallel fetching of multiple packages
- Background refresh tasks

---

## Key Design Decisions

### Why SQLite?

**Requirements:**
- Persist cache across LSP sessions
- Safe access from multiple processes
- Lightweight and single-file deployable

**SQLite Advantages:**
- WAL mode supports concurrent reads
- UNIQUE constraints prevent duplicates
- Transactions guarantee data integrity
- File-based, no additional server required

**Alternatives Considered:**
- In-memory HashMap: Cannot persist across sessions
- Redis: Requires additional server
- JSON file: Risk of corruption with concurrent access

### Trait Objects vs Generics

```rust
// Adopted: Trait objects
pub struct PackageResolver {
    parser: Box<dyn Parser>,
    matcher: Box<dyn VersionMatcher>,
    registry: Box<dyn Registry>,
}

// Not adopted: Generics
pub struct PackageResolver<P: Parser, M: VersionMatcher, R: Registry> {
    parser: P,
    matcher: M,
    registry: R,
}
```

**Reasons for Choosing Trait Objects:**
- `HashMap<RegistryType, PackageResolver>` can store multiple types simultaneously
- Dynamic dispatch overhead is negligible for I/O-bound operations
- Testing with mock implementations is straightforward

### Why Full Document Sync?

LSP has two types of text synchronization:
- **Full**: Entire document sent on each change
- **Incremental**: Only diffs are sent

**Reasons for Choosing Full:**
- Dependency files are typically small (<10KB)
- Implementation is simpler
- Fast enough even with tree-sitter incremental parsing
- Simple model: one change = one diagnostic generation

### Why IndexMap for npm?

```rust
// npm.rs
use indexmap::IndexMap;

let versions: IndexMap<String, serde_json::Value> = ...;
```

**Reasons:**
- npm registry returns versions in publish date order
- `HashMap` does not preserve order
- `IndexMap` preserves insertion order
- Order is important for determining latest version

### Why Regex for go.mod?

Other parsers use tree-sitter, but go.mod uses regex:

```rust
// go_mod.rs
static REQUIRE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"require\s+(\S+)\s+(v[\w\.\-\+]+)").unwrap()
});
```

**Reasons:**
- go.mod format is relatively simple
- tree-sitter's Go grammar doesn't cover go.mod
- Regex provides sufficient accuracy
- Reduces dependencies

### Why Spawn fetch_missing_packages() Asynchronously?

```rust
// backend.rs
tokio::spawn(async move {
    let fetched = fetch_missing_packages(&*storer, registry, &packages).await;
    if !fetched.is_empty() {
        // Re-publish diagnostics
    }
});
```

**Reasons:**
- Display diagnostics immediately when document is opened
- Respond to user without waiting for fetch completion
- Update diagnostics after fetch succeeds

**Flow:**
1. User opens file
2. Immediately display diagnostics for cached packages
3. Fetch uncached packages in background
4. Update diagnostics after fetch completion

---

## Design Patterns

### 1. Registry Type Pattern

The `RegistryType` enum represents the four ecosystems:

```rust
pub enum RegistryType {
    GitHubActions,
    Npm,
    CratesIo,
    GoProxy,
}
```

**Usage:**
- Parser detection from file URI
- Managing resolvers as `HashMap` keys
- Identifying packages in cache

### 2. Composable Resolution Pattern

`PackageResolver` combines three trait objects:

```rust
pub struct PackageResolver {
    parser: Box<dyn Parser>,
    matcher: Box<dyn VersionMatcher>,
    registry: Box<dyn Registry>,
}
```

**Benefits:**
- Each component can be tested independently
- No changes to existing code when adding new registries
- Easy testing through mock injection

### 3. Dual Fetch Mechanism

Two fetching strategies:

| Strategy | Trigger | Purpose |
|----------|---------|---------|
| Background Refresh | Server startup | Update stale cache |
| On-demand Fetch | Document open | Fetch uncached packages |

**Background Refresh:**
- Detect stale packages via `get_packages_needing_refresh()`
- Periodically refresh cache
- Does not block user operations

**On-demand Fetch:**
- Detect uncached packages via `filter_packages_not_in_cache()`
- Fetch only necessary packages when needed
- Runs in background after displaying diagnostics

### 4. Fetch Locking Pattern

Prevents duplicate fetches from multiple processes:

```rust
// cache.rs
pub fn try_start_fetch(&self, registry_type: RegistryType, package_name: &str) -> Result<bool, CacheError> {
    // Update fetching_since if NULL or older than 30 seconds, return true
    // Otherwise return false
}

pub fn finish_fetch(&self, registry_type: RegistryType, package_name: &str) -> Result<(), CacheError> {
    // Set fetching_since to NULL
}
```

**Behavior:**
1. Call `try_start_fetch()` when starting a fetch
2. Lock acquired → execute fetch
3. Lock not acquired → skip (another process is fetching)
4. After fetch completion (success or failure), call `finish_fetch()` to release lock

**Timeout:**
- Locks older than 30 seconds are considered invalid
- Recovers locks from crashed processes

### 5. Staggered Fetch Pattern

Delayed fetching to avoid rate limits:

```rust
// refresh.rs
let futures = packages.into_iter().enumerate().map(|(i, package)| {
    let delay = Duration::from_millis(FETCH_STAGGER_DELAY_MS * i as u64);
    async move {
        sleep(delay).await;
        fetch_and_cache_package(...).await;
    }
});

join_all(futures).await;
```

**Behavior:**
- Start each fetch at 10ms intervals
- Execute in parallel with staggered start times
- Less likely to hit API rate limits

---

## Error Handling Patterns

### Pattern 1: Log and Convert to Option

```rust
let Some(value) = operation()
    .inspect_err(|e| warn!("Operation failed: {}", e))
    .ok()
else {
    return;
};
```

### Pattern 2: Log and Propagate

```rust
operation()
    .inspect_err(|e| error!("Operation failed: {}", e))
    .map_err(CustomError::from)
```

### Pattern 3: Log and Use Default Value

```rust
let result = operation()
    .inspect_err(|e| error!("Fetch failed: {}", e))
    .unwrap_or_default();
```

**Principles:**
- Don't suppress errors; log them
- Don't use `.expect()` (panic prevention)
- Errors that don't need to be shown to users are logged only

---

## Extension Guide

### Adding a New Registry Type

1. **Add RegistryType variant**

```rust
// src/parser/types.rs
pub enum RegistryType {
    GitHubActions,
    Npm,
    CratesIo,
    GoProxy,
    NewRegistry,  // NEW
}

impl RegistryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            // ...
            Self::NewRegistry => "new_registry",
        }
    }
}
```

2. **Create Parser**

```rust
// src/parser/new_format.rs
pub struct NewFormatParser;

impl Parser for NewFormatParser {
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError> {
        // Parsing implementation
    }
}
```

3. **Create VersionMatcher**

```rust
// src/version/matchers/new_registry.rs
pub struct NewRegistryMatcher;

impl VersionMatcher for NewRegistryMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::NewRegistry
    }

    fn version_exists(&self, version_spec: &str, available: &[String]) -> bool {
        // Version matching implementation
    }

    fn compare_to_latest(&self, current: &str, latest: &str) -> CompareResult {
        // Comparison implementation
    }
}
```

4. **Create Registry**

```rust
// src/version/registries/new_registry.rs
pub struct NewRegistryClient { /* ... */ }

#[async_trait]
impl Registry for NewRegistryClient {
    fn registry_type(&self) -> RegistryType {
        RegistryType::NewRegistry
    }

    async fn fetch_all_versions(&self, package_name: &str) -> Result<PackageVersions, RegistryError> {
        // API call implementation
    }
}
```

5. **Update Resolver factory**

```rust
// src/lsp/resolver.rs
pub fn create_default_resolvers(client: reqwest::Client) -> HashMap<RegistryType, PackageResolver> {
    let mut resolvers = HashMap::new();
    // ...
    resolvers.insert(
        RegistryType::NewRegistry,
        PackageResolver::new(
            Box::new(NewFormatParser),
            Box::new(NewRegistryMatcher),
            Box::new(NewRegistryClient::new(client.clone())),
        ),
    );
    resolvers
}
```

6. **Add URI detection**

```rust
// src/parser/types.rs
pub fn detect_parser_type(uri: &str) -> Option<RegistryType> {
    // ...
    if uri.ends_with("new_format.xyz") {
        return Some(RegistryType::NewRegistry);
    }
    // ...
}
```

### Adding Configuration Options

1. **Add to LspConfig**

```rust
// src/config.rs
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LspConfig {
    pub cache: CacheConfig,
    pub registries: RegistriesConfig,
    pub new_option: NewOptionConfig,  // NEW
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NewOptionConfig {
    pub enabled: bool,
    pub value: String,
}
```

2. **Use in Backend**

```rust
// src/lsp/backend.rs
let config = self.config.read().unwrap();
if config.new_option.enabled {
    // ...
}
```

---

## Performance Characteristics

### Cache Operations
- **Time Complexity**: O(1) indexed lookups
- **Space Complexity**: SQLite manages on-disk storage
- **Concurrency**: WAL mode enables concurrent reads

### Fetch Operations
- **Delay**: Requests start at 10ms intervals
- **Parallelism**: Multiple packages fetched in parallel with Tokio
- **Deduplication**: Lock-based prevention of duplicate fetches for same package

### Diagnostic Generation
- **Parsing**: tree-sitter is optimized for incremental parsing
- **Comparison**: Linear scan of available versions (typically <1000)
- **Caching**: On-demand to reduce network calls

---

## Troubleshooting

### Diagnostics Not Appearing

1. **Check Cache**
   - Verify `~/.local/share/version-lsp/versions.db` exists
   - Check if packages are registered using SQLite browser

2. **Check Logs**
   - `~/.local/share/version-lsp/version-lsp.log`
   - Look for parse errors or fetch errors

3. **Check Configuration**
   - Ensure registry is set to `enabled: true`

### Rate Limit Errors

1. **GitHub API**
   - Without authentication: 60 req/hour
   - Set `GITHUB_TOKEN` environment variable

2. **Other Registries**
   - Increase `FETCH_STAGGER_DELAY_MS` (in config.rs)

### Cache Corruption

1. **Delete Database**
   ```bash
   rm ~/.local/share/version-lsp/versions.db
   ```

2. **Restart LSP Server**
   - Database will be recreated on next startup

---

## JSR (JavaScript Registry) Support

### Overview

JSR is the modern JavaScript registry created by Deno, designed for TypeScript-first packages.
This section describes the design for adding JSR support to version-lsp.

### File Format: deno.json

```json
{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@^1.0.1",
    "@std/path": "jsr:@std/path@1.0.0"
  }
}
```

**Characteristics:**
- JSON format (can reuse tree-sitter JSON parser)
- Dependencies in `imports` field
- Format: `"jsr:@scope/package@version"`
- Version follows npm semver specification (^, ~, exact, etc.)

### JSR Registry API

**Endpoint:**
```
https://jsr.io/@{scope}/{package}/meta.json
```

**Example:**
```
https://jsr.io/@luca/flag/meta.json
```

**Response:**
```json
{
  "scope": "luca",
  "name": "flag",
  "latest": "1.0.1",
  "versions": {
    "1.0.0": {},
    "1.0.1": {}
  }
}
```

**Important Headers:**
- `Accept` header must NOT include `text/html`
- Recommended: `Accept: application/json`

### Design Decisions

#### Why Reuse NpmVersionMatcher?

JSR uses the same semver specification as npm:
- Caret (^): `^1.2.3` → `>=1.2.3 <2.0.0`
- Tilde (~): `~1.2.3` → `>=1.2.3 <1.3.0`
- Exact: `1.2.3`
- Comparison: `>=`, `>`, `<=`, `<`

**Decision:** Create `JsrVersionMatcher` that delegates to existing npm semver logic.

```rust
pub struct JsrVersionMatcher;

impl VersionMatcher for JsrVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Jsr
    }

    fn version_exists(&self, version_spec: &str, available: &[String]) -> bool {
        // Reuse npm version matching logic
        npm_version_exists(version_spec, available)
    }

    fn compare_to_latest(&self, current: &str, latest: &str) -> CompareResult {
        npm_compare_to_latest(current, latest)
    }
}
```

#### Parser Implementation

**Parsing Strategy:**
1. Use tree-sitter JSON parser (same as package.json)
2. Extract `imports` field
3. Filter entries with `jsr:` prefix
4. Parse format: `jsr:@scope/package@version`

**Version Extraction:**
```rust
// Input: "jsr:@luca/flag@^1.0.1"
// Output: PackageInfo { name: "@luca/flag", version: "^1.0.1", ... }
fn parse_jsr_specifier(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix("jsr:")?;
    // @scope/package@version format
    // Find the @ that separates package from version (after the scope's @)
    let slash_pos = rest.find('/')?;
    let after_slash = &rest[slash_pos + 1..];
    if let Some(at_pos) = after_slash.find('@') {
        let package_name = &rest[..slash_pos + 1 + at_pos];
        let version = &after_slash[at_pos + 1..];
        Some((package_name.to_string(), version.to_string()))
    } else {
        // No version specified
        Some((rest.to_string(), "latest".to_string()))
    }
}
```

#### Registry Implementation

**Response Structure:**
```json
{
  "scope": "std",
  "name": "path",
  "latest": "1.1.3",
  "versions": {
    "1.0.0": { "createdAt": "2024-01-01T00:00:00.000Z" },
    "1.0.1": { "createdAt": "2024-02-01T00:00:00.000Z", "yanked": true },
    "1.1.0": { "createdAt": "2024-03-01T00:00:00.000Z" }
  }
}
```

**Implementation:**
```rust
pub struct JsrRegistry {
    client: reqwest::Client,
    base_url: String,  // Default: "https://jsr.io"
}

#[derive(Deserialize)]
struct JsrMetaResponse {
    latest: Option<String>,
    versions: HashMap<String, JsrVersionMeta>,
}

#[derive(Deserialize)]
struct JsrVersionMeta {
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    #[serde(default)]
    yanked: bool,
}

impl Registry for JsrRegistry {
    async fn fetch_all_versions(&self, package_name: &str) -> Result<PackageVersions, RegistryError> {
        // package_name: "@luca/flag"
        // URL: https://jsr.io/@luca/flag/meta.json
        let url = format!("{}/{}/meta.json", self.base_url, package_name);

        let response = self.client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await?;

        let meta: JsrMetaResponse = response.json().await?;

        // Filter out yanked versions and sort by createdAt (oldest first)
        let mut versions: Vec<(String, Option<DateTime<Utc>>)> = meta.versions
            .into_iter()
            .filter(|(_, meta)| !meta.yanked)
            .map(|(v, meta)| {
                let timestamp = meta.created_at
                    .and_then(|ts| DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                (v, timestamp)
            })
            .collect();

        versions.sort_by(|(_, a), (_, b)| a.cmp(b));

        let versions: Vec<String> = versions.into_iter().map(|(v, _)| v).collect();

        Ok(PackageVersions::new(versions))
    }
}
```

**Key Points:**
- Sort versions by `createdAt` (oldest first, newest last) - same as npm
- Filter out `yanked` versions (similar to crates.io)
- Use `latest` field from response for dist-tags if needed

### Component Summary

| Component | File | Description |
|-----------|------|-------------|
| RegistryType | `src/parser/types.rs` | Add `Jsr` variant |
| Parser | `src/parser/deno_json.rs` | New: Parse deno.json imports |
| Registry | `src/version/registries/jsr.rs` | New: JSR API client |
| Matcher | `src/version/matchers/jsr.rs` | New: Delegates to npm logic |
| Resolver | `src/lsp/resolver.rs` | Register JSR resolver |

### Implementation Order

Following TDD methodology:

1. **RegistryType extension** (structural change)
   - Add `Jsr` variant
   - Update `as_str()`, `FromStr`, `detect_parser_type()`

2. **Parser implementation** (behavioral change)
   - Write failing tests for deno.json parsing
   - Implement `DenoJsonParser`
   - Handle jsr: prefix extraction

3. **Registry implementation** (behavioral change)
   - Write failing tests with mock server
   - Implement `JsrRegistry`
   - Handle API response parsing

4. **Matcher implementation** (behavioral change)
   - Write failing tests for version matching
   - Implement `JsrVersionMatcher` (delegate to npm logic)

5. **Integration** (behavioral change)
   - Register resolver in factory
   - Integration tests

---

## Future Extension Candidates

### Feature Additions
- [x] JSR (deno.json) support
- [ ] Display version details on Hover
- [ ] Code Action to update to latest version
- [ ] Completion for version candidates
- [ ] Private registry support

### Performance Improvements
- [ ] Incremental document sync
- [ ] Memory layer for cache (in front of SQLite)
- [ ] Batch API calls (for supporting registries only)

### Operational Improvements
- [ ] Metrics collection (fetch time, cache hit rate)
- [ ] Hot reload of configuration file
- [ ] Per-package cache expiration settings
