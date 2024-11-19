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
        devShells.default = with pkgs; mkShell {
          buildInputs = [
            rust-bin.stable."1.82.0".default
            openssl
            pkg-config
            ffmpeg
            git
            protobuf
            gtk4
          ] ++ lib.optional isDarwin (with darwin.apple_sdk.frameworks; [ Security CoreServices ]);
        };
      }
    );
}

