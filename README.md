![GitHub Repo stars](https://img.shields.io/github/stars/skanehira/version-lsp?style=social)
![GitHub](https://img.shields.io/github/license/skanehira/version-lsp)
![GitHub all releases](https://img.shields.io/github/downloads/skanehira/version-lsp/total)
![GitHub CI Status](https://img.shields.io/github/actions/workflow/status/skanehira/version-lsp/ci.yaml?branch=main)
![GitHub Release Status](https://img.shields.io/github/v/release/skanehira/version-lsp)

# version-lsp

A Language Server Protocol (LSP) implementation that provides version checking diagnostics for package dependency files.

## Features

- Detects outdated package versions and shows update suggestions
- Reports errors for non-existent versions
- Supports version ranges (e.g., `^1.0.0`, `~1.0.0`, `>=1.0.0`)
- Caches version information locally for fast response

## Supported Files

| File | Registry |
|------|----------|
| `package.json` | npm |
| `Cargo.toml` | crates.io |
| `go.mod` | Go Proxy |
| `.github/workflows/*.yml` | GitHub Releases |

## Installation

### From GitHub Releases

Download the latest binary from [GitHub Releases](https://github.com/skanehira/version-lsp/releases).

### From Source

```bash
cargo install --git https://github.com/skanehira/version-lsp
```

## Editor Setup

### Neovim (nvim-lspconfig)

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

if not configs.version_lsp then
  configs.version_lsp = {
    default_config = {
      cmd = { 'version-lsp' },
      filetypes = { 'json', 'toml', 'gomod', 'yaml' },
      root_dir = function(fname)
        return lspconfig.util.find_git_ancestor(fname)
      end,
      settings = {},
    },
  }
end

lspconfig.version_lsp.setup({})
```

### VSCode

Create `.vscode/settings.json` in your project:

```json
{
  "version-lsp.enable": true
}
```

Note: VSCode extension is not yet available. Use a generic LSP client extension.

### Helix

Add to `~/.config/helix/languages.toml`:

```toml
[language-server.version-lsp]
command = "version-lsp"

[[language]]
name = "json"
language-servers = ["version-lsp"]

[[language]]
name = "toml"
language-servers = ["version-lsp"]

[[language]]
name = "gomod"
language-servers = ["version-lsp"]

[[language]]
name = "yaml"
language-servers = ["version-lsp"]
```

## Configuration

The LSP server accepts the following configuration options via `workspace/didChangeConfiguration`:

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
    }
  }
}
```

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `cache.refreshInterval` | number | `86400000` | Cache refresh interval in milliseconds (default: 24 hours) |
| `registries.npm.enabled` | boolean | `true` | Enable npm registry checks |
| `registries.crates.enabled` | boolean | `true` | Enable crates.io registry checks |
| `registries.goProxy.enabled` | boolean | `true` | Enable Go Proxy registry checks |
| `registries.github.enabled` | boolean | `true` | Enable GitHub Releases checks |

## Data Storage

version-lsp stores its cache database at:
- Linux/macOS: `$XDG_DATA_HOME/version-lsp/versions.db` or `~/.local/share/version-lsp/versions.db`
- Fallback: `./version-lsp/versions.db`

## License

MIT
