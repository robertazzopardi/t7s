{
  description = "k9s-style TUIs: d7s (PostgreSQL/MySQL/MariaDB/SQLite) and c8s (Docker)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    # x86_64-darwin is left out because nixpkgs 26.11 dropped support for it.
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ] (system:
      let
        overlays = [
          (import rust-overlay)
          # nixpkgs' cargo-llvm-cov fails its own test suite here and is not in
          # the binary cache, so skip the check to make it installable.
          (final: prev: {
            cargo-llvm-cov = prev.cargo-llvm-cov.overrideAttrs (_: {
              doCheck = false;
            });
          })
        ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Keep in sync with `rust-version` in Cargo.toml (the CI `msrv` job).
        # Used for the package builds, which pin exactly that toolchain.
        msrvVersion = "1.89.0";

        # Keep in sync with the nightly pin in `justfile` (`fmt`) and the CI
        # `fmt` job. rustfmt.toml uses nightly-only options.
        nightlyVersion = "2026-09-08";

        # Packages build with the MSRV toolchain: rustc + cargo + std only.
        # `minimal` rather than `default`, which would also drag in ~700 MiB of
        # rust-docs.
        rustBuildToolchain = pkgs.rust-bin.stable.${msrvVersion}.minimal;

        # The devShell tracks latest stable so `just clippy` and `just fmt`
        # see the same lints CI does (CI uses the default toolchain). The MSRV
        # stays enforced by rustBuildToolchain and CI's `msrv` job.
        rustToolchain = pkgs.rust-bin.stable.latest.minimal.override {
          extensions = [ "rustfmt" "clippy" "rust-src" "llvm-tools" ];
        };

        # Just the nightly rustfmt component (rustfmt.toml uses nightly-only
        # options), not the whole nightly toolchain.
        rustNightly = pkgs.rust-bin.nightly.${nightlyVersion}.rustfmt;

        # Match the dev toolchain
        rustAnalyzer = pkgs.rust-bin.stable.latest.rust-analyzer;

        # Libraries the crates link against. Only libdbus-1 is needed (used by
        # dbus-secret-service for the keyring): rusqlite is built with `bundled`
        # (static SQLite), tokio-postgres is pure Rust and TLS uses rustls.
        packageInputs = with pkgs; [
          dbus
        ];

        # Build-time tools. pkg-config has to live here rather than above:
        # rustPlatform sets strictDeps, which keeps buildInputs off PATH and
        # skips their pkg-config setup hook.
        nativeTools = with pkgs; [
          pkg-config
        ];

        # Dev-only tools. clang/mold are what ~/.cargo/config.toml links with
        # (`linker = "clang"`, `-C link-arg=-fuse-ld=mold`), so `cargo build`
        # needs both. None of this belongs in `mkCrate`: the package sandbox
        # sees no cargo config and only needs rustc/cargo.
        devTools = with pkgs; [
          just
          clang
          mold
          cargo-llvm-cov
        ];

        crateVersion = name:
          (builtins.fromTOML (builtins.readFile ./crates/${name}/Cargo.toml)).package.version;

        # Build with the pinned toolchain rather than nixpkgs' own rustc/cargo,
        # which would drag a second toolchain and LLVM into every build.
        mkCrate = name: (pkgs.makeRustPlatform {
          cargo = rustBuildToolchain;
          rustc = rustBuildToolchain;
        }).buildRustPackage {
          pname = name;
          version = crateVersion name;

          # Only the files the packages are built from, so editing flake.nix
          # (or anything else outside this set) does not invalidate the build.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
            ];
          };

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          buildInputs = packageInputs;

          # rustBuildToolchain first so rustc/cargo are the ones on PATH.
          nativeBuildInputs = [ rustBuildToolchain ] ++ nativeTools;

          # Only build/install the requested member of the workspace.
          cargoBuildFlags = [ "--package" name ];
          cargoInstallFlags = [ "--package" name ];

          # Don't run tests during build (optional, remove if you want tests)
          doCheck = false;
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = packageInputs ++ nativeTools ++ devTools ++ [
            rustToolchain
            rustAnalyzer
          ];

          # Nightly bin for `just fmt` / `just fmt-check` (rustfmt.toml uses nightly-only options)
          RUST_NIGHTLY_BIN = "${rustNightly}/bin";

          # So the compiled binaries can find libdbus-1 at runtime.
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath packageInputs}";

          # dbus-1.pc lives in dbus.dev.
          PKG_CONFIG_PATH = "${pkgs.dbus.dev}/lib/pkgconfig";

          # For rust-analyzer's std source navigation.
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "d7s development environment (rustc $(rustc --version | cut -d' ' -f2))"
            echo "Run \`just\` for the project commands"
          '';
        };

        packages = {
          default = self.packages.${system}.d7s;
          d7s = mkCrate "d7s";
          c8s = mkCrate "c8s";
        };

        # Formatter configuration
        formatter = pkgs.nixpkgs-fmt;
      });
}
