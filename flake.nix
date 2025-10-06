{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      pkgs = import nixpkgs {
        system = "x86_64-linux";
        overlays = [ (import rust-overlay) ];
      };
    in
    {
      devShells.x86_64-linux.default = pkgs.mkShell.override { stdenv = pkgs.clangStdenv; } rec {
        buildInputs = with pkgs; [
          pkg-config
          libxkbcommon
          wayland
          gtk3
          glib
          libGL
          libepoxy
          fontconfig

          clang-tools
          llvmPackages_latest.clang
          llvmPackages_latest.lldb
          cmake
          ninja

          vulkan-loader
          (rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
            ];
          }))
          rustPlatform.bindgenHook
        ];

        shellHook = ''
          runHook bindgenHook
          export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${builtins.toString (pkgs.lib.makeLibraryPath buildInputs)}:/home/idkana/project/wayflutter/engine"
          export PATH="$HOME/flutter/bin:$PATH"
        '';
      };
    };
}
