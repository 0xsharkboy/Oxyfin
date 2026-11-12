{
  description = "Oxyfin, a native Jellyfin desktop client written in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ]
      (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          lib = pkgs.lib;
          linuxRuntimeLibs = with pkgs; [
            libGL
            wayland
            libxkbcommon
            libx11
            libxcursor
            libxi
            libxrandr
            fontconfig
            freetype
          ];
        in
        {
          devShells.default = pkgs.mkShell (
            {
              packages =
                with pkgs;
                [
                  rustc
                  cargo
                  clippy
                  rustfmt
                  rust-analyzer
                  pkg-config
                  mpv-unwrapped
                ]
                ++ lib.optionals stdenv.hostPlatform.isLinux linuxRuntimeLibs;

              shellHook = ''
                git config core.hooksPath .githooks
              '';
            }
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              # winit and glutin load these libraries with dlopen at runtime.
              LD_LIBRARY_PATH = "/run/opengl-driver/lib:" + lib.makeLibraryPath linuxRuntimeLibs;
            }
          );

          formatter = pkgs.nixpkgs-fmt;
        }
      );
}
