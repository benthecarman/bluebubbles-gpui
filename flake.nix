{
  description = "BlueBubbles GPUI: a native Linux client for the BlueBubbles Server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        # Loaded with dlopen by GPUI and its dependencies, so they are not
        # linked and must be on the library path at run time.
        runtimeLibs = with pkgs; [
          wayland
          vulkan-loader
          libxcb
          libxkbcommon
          libx11
          libxcursor
          libxi
          fontconfig
          freetype
          dbus
        ];
        craneLib = crane.mkLib pkgs;
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            let base = baseNameOf path; in
            !(builtins.elem base [ "target" "target-deb" "dist" ".claude" "docs" "bluebubbles-app" "bluebubbles-server" ]);
        };
        commonArgs = {
          pname = "bluebubbles-gpui";
          version = "0.1.0";
          inherit src;
          strictDeps = true;
          cargoExtraArgs = "--locked -p bluebubbles-gpui";
          # The tests run in the dev shell; the package build skips them.
          doCheck = false;
          nativeBuildInputs = with pkgs; [ pkg-config cmake perl makeWrapper ];
          buildInputs = runtimeLibs ++ (with pkgs; [ openssl ]);
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      in
      {
        packages.default = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          postInstall = ''
            wrapProgram $out/bin/bluebubbles-gpui \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
            install -Dm644 packaging/linux/bluebubbles-gpui.desktop \
              $out/share/applications/bluebubbles-gpui.desktop
            for size in 128 256 512; do
              install -Dm644 assets/icons/bluebubbles-$size.png \
                $out/share/icons/hicolor/''${size}x''${size}/apps/bluebubbles-gpui.png
            done
          '';
          meta.mainProgram = "bluebubbles-gpui";
        });

        apps.default = flake-utils.lib.mkApp { drv = self.packages.${system}.default; };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ pkg-config cmake perl rustc cargo clippy rustfmt rust-analyzer ];
          buildInputs = runtimeLibs ++ (with pkgs; [ openssl vulkan-headers ]);
          env = {
            RUST_BACKTRACE = "1";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          };
        };
      });
}
