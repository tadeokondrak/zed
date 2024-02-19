let
pkgs = import <nixpkgs> {};
lib = pkgs.lib;
in
pkgs.mkShell rec {
  buildInputs = with pkgs; [
    openssl
    fontconfig
    protobuf
    alsa-lib
    wayland
    libxkbcommon
    libGL
    libdrm
    libelf
    xorg.libxcb
    glslang
    vulkan-headers
    vulkan-loader
    vulkan-validation-layers
    vulkan-tools
    xorg.libX11
    xorg.libXi
    xorg.libXcursor
    xorg.libXrandr
    xorg.libXext
    xorg.libxshmfence
    xorg.libXxf86vm
    wayland-protocols
    udev
  ];
  nativeBuildInputs = with pkgs; [
    pkg-config
  ];
  RUST_BACKTRACE = "1";
  LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
  VK_LAYER_PATH = "${pkgs.vulkan-validation-layers}/share/vulkan/explicit_layer.d";
}
