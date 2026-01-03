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

          rustToolchainToml = pkgs.lib.importTOML ./rust-toolchain.toml;
          rustVersion = rustToolchainToml.toolchain.channel;

          # Minimal toolchain (for build)
          rustToolchainMinimal = pkgs.rust-bin.stable.${rustVersion}.minimal;
          craneLibMinimal = (crane.mkLib pkgs).overrideToolchain rustToolchainMinimal;

          # Full toolchain from rust-toolchain.toml (for dev)
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          commonArgs = {
            src = (crane.mkLib pkgs).cleanCargoSource ./.;
          };

          cargoArtifacts = craneLibMinimal.buildDepsOnly commonArgs;

          meta = {
            description = "LSP for package version management";
            homepage = "https://github.com/skanehira/version-lsp";
            license = pkgs.lib.licenses.mit;
          };
        in
        {
          packages = {
            default = craneLibMinimal.buildPackage (commonArgs // {
              inherit cargoArtifacts meta;
            });

            # CI build (skip tests)
            ci = craneLibMinimal.buildPackage (commonArgs // {
              inherit cargoArtifacts meta;
              doCheck = false;
            });
          };

          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.cargo-nextest
              pkgs.cargo-llvm-cov
            ];

            RUST_BACKTRACE = 1;
          };
        };
    };
}
