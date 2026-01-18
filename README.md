![GitHub Repo stars](https://img.shields.io/github/stars/skanehira/version-lsp?style=social)
![GitHub](https://img.shields.io/github/license/skanehira/version-lsp)
![GitHub all releases](https://img.shields.io/github/downloads/skanehira/version-lsp/total)
![GitHub CI Status](https://img.shields.io/github/actions/workflow/status/skanehira/version-lsp/ci.yaml?branch=main)
![GitHub Release Status](https://img.shields.io/github/v/release/skanehira/version-lsp)

# version-lsp

A Language Server Protocol (LSP) implementation that provides version checking diagnostics for package dependency files.

<table>
  <tr>
    <td><a href="https://gyazo.com/3c0c5fc42d0109033eb2017254135fcf"><img src="https://i.gyazo.com/3c0c5fc42d0109033eb2017254135fcf.png" alt="Image from Gyazo"></a></td>
    <td><a href="https://gyazo.com/e34f0eebacafc65bd06761cee1ffe5de"><img src="https://i.gyazo.com/e34f0eebacafc65bd06761cee1ffe5de.png" alt="Image from Gyazo"></a></td>
  </tr>
  <tr>
    <td><a href="https://gyazo.com/2458d2eb4966c9c2dea30eafcfd8ff2b"><img src="https://i.gyazo.com/2458d2eb4966c9c2dea30eafcfd8ff2b.png" alt="Image from Gyazo"></a></td>
    <td><a href="https://gyazo.com/b81a1ac9817c31398013e01a013c6d08"><img src="https://i.gyazo.com/b81a1ac9817c31398013e01a013c6d08.png" alt="Image from Gyazo"></a></td>
  </tr>
</table>

## Features

- Detects outdated package versions and shows update suggestions
- Reports errors for non-existent versions
- Supports version ranges (e.g., `^1.0.0`, `~1.0.0`, `>=1.0.0`)
- Caches version information locally for fast response

## Supported Files

| File                                                  | Registry        |
| ----------------------------------------------------- | --------------- |
| `package.json`                                        | npm             |
| `pnpm-workspace.yaml`                                 | npm             |
| `Cargo.toml`                                          | crates.io       |
| `go.mod`                                              | Go Proxy        |
| `.github/workflows/*.yaml`/`.github/actions/*/*.yaml` | GitHub Releases |
| `deno.json` / `deno.jsonc`                            | JSR             |

### pnpm Catalogs

Supports [pnpm catalogs](https://pnpm.io/catalogs) defined in `pnpm-workspace.yaml`:

```yaml
# Single catalog
catalog:
  react: ^18.2.0
  lodash: ^4.17.21

# Named catalogs
catalogs:
  react17:
    react: ^17.0.2
  react18:
    react: ^18.2.0
```

## Installation

### From GitHub Releases

Download the latest binary from [GitHub Releases](https://github.com/skanehira/version-lsp/releases).

### From Source

```bash
cargo install --git https://github.com/skanehira/version-lsp
```

### Using Nix Flake

If you have Nix with flakes enabled:

```bash
# Enter development shell with Rust toolchain
nix develop

# Build the package
nix build

# Run directly from flake
nix run github:skanehira/version-lsp
```

## Editor Setup

### Neovim (vim.lsp)

Available in Neovim >= 0.11

```lua
vim.lsp.config('version_lsp', {
  cmd = { 'version-lsp' },
  filetypes = { 'json', 'jsonc', 'toml', 'gomod', 'yaml' },
  root_markers = { '.git' },
  settings = {
    ["version-lsp"] = {
      -- See 'Configuration Options' section below for details
    },
  },
})

vim.lsp.enable('version_lsp')
```

### Neovim (nvim-lspconfig)

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

if not configs.version_lsp then
  configs.version_lsp = {
    default_config = {
      cmd = { 'version-lsp' },
      filetypes = { 'json', 'jsonc', 'toml', 'gomod', 'yaml' },
      root_dir = function(fname)
        return lspconfig.util.find_git_ancestor(fname)
      end,
      settings = {},
    },
  }
end

lspconfig.version_lsp.setup({
  settings = {
    ["version-lsp"] = {
      cache = {
        refreshInterval = 86400000,  -- 24 hours (milliseconds)
      },
      registries = {
        npm = { enabled = true },
        crates = { enabled = true },
        goProxy = { enabled = true },
        github = { enabled = true },
        pnpmCatalog = { enabled = true },
        jsr = { enabled = true },
      },
      ignorePrerelease = true,  -- Ignore prerelease versions (default: true)
    },
  },
})
```

### Configuration Options

| Option                           | Type    | Default    | Description                                                |
| -------------------------------- | ------- | ---------- | ---------------------------------------------------------- |
| `cache.refreshInterval`          | number  | `86400000` | Cache refresh interval in milliseconds (default: 24 hours) |
| `registries.npm.enabled`         | boolean | `true`     | Enable npm registry checks                                 |
| `registries.crates.enabled`      | boolean | `true`     | Enable crates.io registry checks                           |
| `registries.goProxy.enabled`     | boolean | `true`     | Enable Go Proxy registry checks                            |
| `registries.github.enabled`      | boolean | `true`     | Enable GitHub Releases checks                              |
| `registries.pnpmCatalog.enabled` | boolean | `true`     | Enable pnpm catalog checks                                 |
| `registries.jsr.enabled`         | boolean | `true`     | Enable JSR registry checks                                 |
| `ignorePrerelease`               | boolean | `true`     | Ignore prerelease versions (alpha, beta, rc, etc.)         |

## Data Storage

version-lsp stores its cache database at:
- Linux/macOS: `$XDG_DATA_HOME/version-lsp/versions.db` or `~/.local/share/version-lsp/versions.db`
- Fallback: `./version-lsp/versions.db`

## License

MIT
