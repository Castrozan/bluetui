{ pkgs, ... }:

{
  packages = with pkgs; [
    rustc
    cargo
    clippy
    rustfmt
    pkg-config
    dbus
    cmake
  ];

  env.LD_LIBRARY_PATH = "${pkgs.dbus.lib}/lib";
}
