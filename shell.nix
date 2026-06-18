{
  pkgs ? import <nixpkgs> { },
}:

with pkgs;
mkShell {
  shellHook = ''
    export LIBCLANG_PATH="${libclang.lib}/lib"
  '';

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    glib
    clang
    openssl
    libclang
  ];
}
