{
  description = "sqlx-cloudflare - sqlx drivers for Cloudflare Workers.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # From rust-toolchain.toml, so the shell and a plain `cargo` outside it
        # are the same compiler -- and, as importantly, so the shell gets the
        # wasm32-unknown-unknown target that file names. nixpkgs' own Rust has
        # no `core` for it, and every wasm build would fail with "can't find
        # crate for `core`".
        rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        devShells.default = pkgs.mkShell {
          name = "sqlx-cloudflare";

          packages = [
            rust-toolchain
            # The integration tests: `worker-build` turns the test Worker into
            # a wasm module and its JavaScript shim, and `wrangler dev --local`
            # serves it against a local D1 -- no Cloudflare account involved.
            pkgs.worker-build
            pkgs.wrangler
            pkgs.nodejs
          ];
        };
      }
    );
}
