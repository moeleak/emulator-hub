{ lib, rustPlatform, pkg-config, protobuf, makeWrapper, stdenv, libiconv, alsa-lib,
  libxkbcommon, wayland, libGL, vulkan-loader, xorg }:
let
  runtimeLibs = [ alsa-lib libxkbcommon wayland libGL vulkan-loader xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr ];
in rustPlatform.buildRustPackage {
  pname = "emulator-hub";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;
  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type:
      lib.cleanSourceFilter path type
      && !(builtins.elem (baseNameOf path) [ "target" "dist" ".direnv" ]);
  };
  cargoLock = {
    lockFile = ../Cargo.lock;
    allowBuiltinFetchGit = true;
  };
  cargoBuildFlags = [ "-p" "emulator-hub" ];
  nativeBuildInputs = [ pkg-config protobuf makeWrapper ];
  buildInputs = lib.optionals stdenv.isLinux runtimeLibs ++ lib.optionals stdenv.isDarwin [ libiconv ];
  PROTOC = "${protobuf}/bin/protoc";
  postInstall = ''
    install -Dm644 LICENSE $out/share/licenses/emulator-hub/LICENSE
    install -Dm644 THIRD_PARTY_NOTICES.txt $out/share/licenses/emulator-hub/THIRD_PARTY_NOTICES.txt
    install -Dm644 crates/hub-app/assets/fonts/OFL-1.1.txt $out/share/licenses/emulator-hub/FONT-OFL-1.1.txt
  '' + lib.optionalString stdenv.isLinux ''
    wrapProgram $out/bin/emulator-hub --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
    install -Dm644 packaging/emulator-hub.desktop $out/share/applications/emulator-hub.desktop
    install -Dm644 packaging/emulator-hub.svg $out/share/icons/hicolor/scalable/apps/emulator-hub.svg
  '';
  meta = {
    description = "Material Design desktop hub for Android Emulator and LineageOS";
    homepage = "https://github.com/moeleak/emulator-hub";
    license = lib.licenses.asl20;
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
    mainProgram = "emulator-hub";
  };
}
