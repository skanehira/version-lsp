# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Build & Run
```bash
# Build
cargo build

# Release build
cargo build --release

# Run (example)
cargo run -- --name "World"
```

### Testing
```bash
# Run all tests
cargo test

# Fast test execution with cargo-nextest (recommended)
cargo nextest run

# Run a single test
cargo test test_name

# Generate coverage (requires cargo-llvm-cov)
cargo llvm-cov nextest --lcov --output-path lcov.info
```

### Quality Checks
```bash
# Format check
cargo fmt -- --check

# Apply formatting
cargo fmt

# Static analysis with Clippy
cargo clippy
```

### Benchmarks
```bash
# Requires nightly toolchain
cargo +nightly bench
```

## Architecture

### Project Structure
- **cargo-generate template**: This repository is a template for generating new Rust CLI projects
- **CLI parser**: Uses clap v4 derive macros for command-line argument processing
- **Benchmarks**: Located in `benches/` directory, uses nightly compiler's test crate

### CI/CD Configuration
- **ci.yaml**: Main CI workflow
  - Runs formatting, Clippy, build, and tests
  - Generates coverage on Linux with octocov reporting
  - Automatic PR feedback via reviewdog
- **benchmark.yaml**: Auto-deploys benchmark results to GitHub Pages
- **audit.yaml**: Security audit for dependencies
- **release.yaml**: Automated release on tag push (cross-platform builds via GoReleaser)

### Key Settings
- **Rust version**: Fixed to 1.92.0 in `rust-toolchain.toml`
- **Edition**: Uses Rust 2024 edition
- **Test tools**: cargo-nextest and cargo-llvm-cov recommended