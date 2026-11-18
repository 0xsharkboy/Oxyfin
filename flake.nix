{
  description = "Oxyfin, a native Jellyfin desktop client written in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    ,
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
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
          appId = "io.github._0xsharkboy.Oxyfin";
          oxyfin = pkgs.rustPlatform.buildRustPackage {
            pname = "oxyfin";
            version = (lib.importTOML ./Cargo.toml).workspace.package.version;

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
                ./data
              ];
            };

            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [ "--package" "oxyfin" ];

            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs =
              with pkgs;
              [
                # rustls links it instead of building its own copy.
                aws-lc
                mpv-unwrapped
                fontconfig
                freetype
              ]
              ++ lib.optionals stdenv.hostPlatform.isLinux (linuxRuntimeLibs ++ [ dbus ]);

            # CI runs the test suite with cargo; running it here would double the build.
            doCheck = false;

            postInstall = ''
              install -Dm644 data/${appId}.desktop -t $out/share/applications
              install -Dm644 data/${appId}.svg -t $out/share/icons/hicolor/scalable/apps
              install -Dm644 data/${appId}.metainfo.xml -t $out/share/metainfo
            '';

            # winit and glutin load these libraries with dlopen at runtime.
            postFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              patchelf --add-rpath ${lib.makeLibraryPath linuxRuntimeLibs} $out/bin/oxyfin
            '';

            meta = {
              description = "Native Jellyfin desktop client";
              homepage = "https://github.com/0xsharkboy/Oxyfin";
              license = lib.licenses.gpl3Only;
              mainProgram = "oxyfin";
              platforms = lib.platforms.linux ++ lib.platforms.darwin;
            };
          };
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
                  jq
                  ffmpeg-headless
                  cargo-deny
                  slint-tr-extractor
                  gettext
                ]
                ++ lib.optionals stdenv.hostPlatform.isLinux (linuxRuntimeLibs ++ [ dbus ]);

              shellHook = ''
                git config core.hooksPath .githooks
              '';
            }
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              # winit and glutin load these libraries with dlopen at runtime.
              LD_LIBRARY_PATH = "/run/opengl-driver/lib:" + lib.makeLibraryPath linuxRuntimeLibs;
            }
          );

          packages = {
            default = oxyfin;
            inherit oxyfin;
          };

          apps.default = flake-utils.lib.mkApp { drv = oxyfin; };

          formatter = pkgs.nixpkgs-fmt;
        }
      );
}
