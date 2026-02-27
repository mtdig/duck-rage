{
  description = "duck-rage DuckDB extension dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/2fc6539b";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Python is only needed for configure_helper.py and the test runner.
        # duckdb itself (==1.4.4) is pip-installed into the Makefile-managed
        # venv at build time, so we don't need to provide it here.
        pythonEnv = pkgs.python3.withPackages (ps: [
          ps.pip
          ps.packaging
        ]);
      in
      {
        devShells.default = pkgs.mkShell {
          name = "duck-rage";

          packages = [
            # Rust toolchain (stable, matching what Cargo.toml expects)
            pkgs.rust-bin.stable.latest.default

            # Python for configure_helper.py and the test runner.
            # duckdb==1.4.4 is pip-installed into the venv by the Makefile.
            pythonEnv

            # DuckDB 1.4.4 CLI (from pinned nixpkgs commit 2fc6539b)
            pkgs.duckdb

            # C/C++ toolchain – provides libstdc++.so.6 needed by the pip duckdb wheel
            pkgs.gcc
            pkgs.stdenv.cc.cc.lib

            # Build essentials
            pkgs.gnumake
            pkgs.pkg-config
            pkgs.openssl
            pkgs.git
          ];

          # Make libstdc++.so.6 discoverable by the Python duckdb wheel
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.stdenv.cc.cc.lib
          ];

          shellHook = ''
            echo "duck-rage dev shell ready"
            echo "  Rust: $(rustc --version)"
            echo "  Python: $(python3 --version)"
          '';
        };
      }
    );
}
