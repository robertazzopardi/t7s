{
  description = "d7s - Database client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Use Rust stable toolchain - pinned to version 1.91.0
        rustToolchain = pkgs.rust-bin.stable."1.91.0".default.override {
          extensions = [ "rustfmt" "clippy" "rust-src" "llvm-tools" ];
        };

        # Nightly toolchain for rustfmt (rustfmt.toml uses nightly-only options)
        rustNightly = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rustfmt" ];
        };

        # System dependencies needed for the project
        buildInputs = with pkgs; [
          just
          cargo-llvm-cov
          # DBus for Linux secret-service support (keyring crate)
          dbus
          # OpenSSL for secret-service encryption (if needed)
          openssl
          # pkg-config for finding libraries
          pkg-config
          # SQLite for rusqlite crate
          sqlite
          # PostgreSQL client libraries for tokio-postgres
          postgresql.lib
          # Clang for building the Rust project
          clang
          # Mold for faster builds
          mold
        ];

        # Libraries needed at runtime
        runtimeLibs = with pkgs; [
          sqlite
          openssl
          dbus
          postgresql.lib
        ];

        # Native build inputs (rustToolchain already includes cargo, rustfmt, clippy)
        # Pin rust-analyzer to match Rust version
        rustAnalyzer = pkgs.rust-bin.stable."1.91.0".rust-analyzer;
        nativeBuildInputs = with pkgs; [
          rustToolchain
          rustAnalyzer
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;
          
          # Nightly bin for `just fmt` / `just fmt-check` (rustfmt.toml uses nightly-only options)
          RUST_NIGHTLY_BIN = "${rustNightly}/bin";
          
          # Make libraries available at runtime
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath runtimeLibs}";

          shellHook = ''
            echo "d7s development environment"
            echo "Rust version: $(rustc --version)"
            echo "Cargo version: $(cargo --version)"
            echo "Run \`just\` for project commands"
          '';

          # Set environment variables for Rust crates that need them
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          
          # For DBus secret-service on Linux
          PKG_CONFIG_PATH = "${pkgs.dbus.lib}/lib/pkgconfig:${pkgs.openssl.dev}/lib/pkgconfig";
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "d7s";
          version = (builtins.fromTOML (builtins.readFile ./crates/d7s/Cargo.toml)).package.version;
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          inherit buildInputs nativeBuildInputs;

          # Don't run tests during build (optional, remove if you want tests)
          doCheck = false;
        };

        # Formatter configuration
        formatter = pkgs.nixpkgs-fmt;
      });
}

