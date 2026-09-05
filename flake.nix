{
  description = "Material Design desktop hub for Android Emulator and LineageOS AVD";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
        linuxLibs = with pkgs; [ alsa-lib libxkbcommon wayland libGL vulkan-loader xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr ];
        nativeApp = pkgs.callPackage ./nix/package.nix { };
        dev = with pkgs; [ rustc cargo rustfmt clippy rust-analyzer pkg-config protobuf python3 git gh curl unzip zip cmake ninja ];
      in {
        devShells.default = pkgs.mkShell {
          packages = dev ++ lib.optionals pkgs.stdenv.isLinux (linuxLibs ++ [ pkgs.android-tools ])
            ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv pkgs.librsvg ];
          LD_LIBRARY_PATH = lib.optionalString pkgs.stdenv.isLinux (lib.makeLibraryPath linuxLibs);
          LIBRARY_PATH = lib.optionalString pkgs.stdenv.isDarwin "${pkgs.libiconv}/lib";
          PROTOC = "${pkgs.protobuf}/bin/protoc";
        };
        devShells.engine = pkgs.mkShell {
          packages = with pkgs; [ git git-repo gnupg cacert python3 cmake ninja ccache curl unzip zip pkg-config ];
        };
        devShells.android = pkgs.mkShell {
          packages = with pkgs; [ git git-repo gnupg cacert jdk17 python3 rsync ccache ];
        };
        packages = {
          default = if pkgs.stdenv.isLinux then pkgs.callPackage ./nix/runtime.nix { app = nativeApp; } else nativeApp;
          native = nativeApp;
          emulator-hub = self.packages.${system}.default;
        } // lib.optionalAttrs pkgs.stdenv.isLinux {
          android-env = pkgs.callPackage ./nix/android-fhs.nix { };
          runtime = pkgs.callPackage ./nix/runtime.nix { };
        };
        apps.default = flake-utils.lib.mkApp { drv = self.packages.${system}.default; };
      });
}
