{ pkgs }:
pkgs.buildFHSEnv {
  name = "lineageos-build-env";
  targetPkgs = p: with p; [
    bash coreutils findutils gnugrep gnused gawk diffutils patch which file git git-repo gnupg
    curl wget cacert rsync unzip zip xz bzip2 gzip gnutar python3 perl jdk17
    gnumake ninja cmake gcc clang pkg-config ccache bc bison flex m4 gperf
    glibc zlib zlib.dev ncurses ncurses5 openssl libxml2 lz4 lzop dtc
    lsb-release libpng libjpeg libxml2 libxslt util-linux procps patchelf libxcrypt-legacy alsa-lib libGL libdrm
    icu krb5 lttng-ust libunwind
    xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr
  ];
  multiPkgs = p: with p; [ zlib ncurses5 ];
  runScript = "bash";
  profile = ''
    export SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt
    export GIT_SSL_CAINFO=/etc/ssl/certs/ca-bundle.crt
    export LANG=C.UTF-8
    export LC_ALL=C.UTF-8
  '';
}
