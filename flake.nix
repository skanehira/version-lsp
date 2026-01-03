{
  description = "version-lsp - LSP for package version management";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, crane, rust-overlay, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem =
        { system, ... }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          src = craneLib.cleanCargoSource ./.;

          commonArgs = {
            inherit src;
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          packages.default = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;

            meta = {
              description = "LSP for package version management";
              homepage = "https://github.com/skanehira/version-lsp";
              license = pkgs.lib.licenses.mit;
            };
          });

          devShells.default = craneLib.devShell {
            packages = [
              pkgs.cargo-nextest
              pkgs.cargo-llvm-cov
            ];

            RUST_BACKTRACE = 1;
          };
        };
    };
}
