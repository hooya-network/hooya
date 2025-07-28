{
  description = "hooya";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        isDarwin = pkgs.stdenv.isDarwin;
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "hooya";
          version = "0.1.2";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            protobuf
          ];

          buildInputs = with pkgs; [
            openssl
            ffmpeg
          ] ++ lib.optionals isDarwin (with darwin.apple_sdk.frameworks; [ Security CoreServices ]);
        };

        devShells.default = with pkgs; mkShell {
          buildInputs = [
            (rust-bin.stable."1.88.0".default.override {
              extensions = [ "rust-src" ];
            })
            openssl
            pkg-config
            ffmpeg
            git
            protobuf
            gtk4
          ] ++ lib.optional isDarwin (with darwin.apple_sdk.frameworks; [ Security CoreServices ]);
          RUST_SRC_PATH="${pkgs.rust-bin.stable."1.88.0".default}/lib/rustlib/src/rust/library";
        };
      }
    );
}

