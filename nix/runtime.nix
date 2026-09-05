{ pkgs, app ? null }:
pkgs.buildFHSEnv {
  name = if app == null then "emulator-hub-runtime" else "emulator-hub";
  targetPkgs = p: with p; [
    bash coreutils cacert glibc stdenv.cc.cc.lib zlib bzip2 expat openssl libxcrypt-legacy
    libGL libglvnd libdrm libbsd libmd libpng libjpeg libxml2 libxslt lsb-release vulkan-loader alsa-lib libpulseaudio fontconfig freetype
    libxkbcommon wayland libusb1 libuuid nss nspr ncurses5
    libx11 libxcursor libxi libxrandr libxrender libxtst libice libsm libxcb
  ] ++ pkgs.lib.optional (app != null) app;
  runScript = if app == null then "bash" else pkgs.writeShellScript "emulator-hub-start" ''
    exec ${app}/bin/emulator-hub "$@"
  '';
  extraInstallCommands = pkgs.lib.optionalString (app != null) ''
    mkdir -p "$out/share"
    cp -r ${app}/share/applications "$out/share/"
    cp -r ${app}/share/icons "$out/share/"
    cp -r ${app}/share/licenses "$out/share/"
  '';
  meta = {
    description = "Emulator Hub runtime with Linux ELF support for downloaded Android tools";
    mainProgram = if app == null then "emulator-hub-runtime" else "emulator-hub";
  };
}
