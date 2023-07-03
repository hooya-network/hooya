let
  moz_overlay = import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz);
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
  binaryen = nixpkgs.stdenv.mkDerivation {
    name = "binaryen";
    src = builtins.fetchTarball {
      url = "https://github.com/WebAssembly/binaryen/releases/download/version_109/binaryen-version_109-x86_64-linux.tar.gz";
    };
    phases = ["installPhase" "patchPhase"];
    installPhase = ''
      mkdir -p $out/bin
      cp $src/bin/* $out/bin
      chmod +x $out/bin/*
    '';
  };
  rust = (nixpkgs.rustChannelOf {
    rustToolchain = ./rust-toolchain;
  }).rust.override {
    extensions = [ "rust-src" ];
  };
in
with nixpkgs;
stdenv.mkDerivation {
  name = "hooya";
  nativeBuildInputs = [
    openssl
    pkg-config
  ];
  buildInputs = [
    rust
    nodejs
    git
    protobuf
    wasm-bindgen-cli
    nodePackages.typescript
    binaryen
    gtk4
  ];

  RUST_SRC_PATH = "${rustPlatform.rustLibSrc}";
  CARGO_NET_GIT_FETCH_WITH_CLI = "true";
}
