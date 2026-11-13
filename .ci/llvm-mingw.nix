# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
#
# llvm-mingw (clang, lld and the mingw-w64 UCRT runtime) for the Windows release
# targets x86_64-pc-windows-gnullvm and aarch64-pc-windows-gnullvm. nixpkgs has no
# such package, and Zig's bundled mingw-w64 lacks the `synchronization` import
# library that Rust's standard library needs on arm64. This is the upstream release
# archive, pinned by hash and only unpacked and patched for the Nix loader: nothing
# is compiled. All of its licences are free (LLVM Apache-2.0 with exceptions,
# mingw-w64 ZPL/public domain); no Microsoft SDK is involved.
#
# Use:  nix build --no-link --print-out-paths --expr \
#         '(builtins.getFlake "<pinned nixpkgs>").legacyPackages.x86_64-linux.callPackage ./.ci/llvm-mingw.nix {}'
{ stdenv, fetchurl, autoPatchelfHook, zlib, zstd }:
stdenv.mkDerivation rec {
  pname = "llvm-mingw";
  version = "20261104";
  src = fetchurl {
    url = "https://github.com/mstorsjo/llvm-mingw/releases/download/${version}/llvm-mingw-${version}-ucrt-ubuntu-22.04-x86_64.tar.xz";
    hash = "sha256-kZOAQD3ofco8WBwgUyMM0pivXokShmKLoGE7f+03CSY=";
  };
  nativeBuildInputs = [ autoPatchelfHook ];
  # libstdc++ and libgcc_s; glibc is the stdenv's, libLLVM and libclang-cpp ship in lib/.
  buildInputs = [ stdenv.cc.cc.lib zlib zstd ];
  dontConfigure = true;
  dontBuild = true;
  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -r . "$out"
    # The debugger is not part of any build and would pull libxml2, xz and ncurses in.
    rm -f "$out"/bin/lldb* "$out"/lib/liblldb*
    runHook postInstall
  '';
  # The archive holds Windows objects and import libraries next to the Linux tools.
  dontStrip = true;
  meta.description = "LLVM-based mingw-w64 toolchain (upstream release, patched for the Nix loader)";
}
