# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
#
# llvm-mingw (clang, lld and the mingw-w64 UCRT runtime) for the Windows release
# targets x86_64-pc-windows-gnullvm and aarch64-pc-windows-gnullvm. nixpkgs has no
# such package, and Zig's bundled mingw-w64 lacks the `synchronization` import
# library that Rust's standard library needs on arm64. The source is the upstream
# release archive, but NOT fetched here: it is the flake input `llvm-mingw-dist`
# (flake.nix, pinned in flake.lock), so `nix flake archive` pushes its NAR to the
# srv Celler ci cache on every CI run and a GitHub outage never blocks the build.
# The archive is only unpacked and patched for the Nix loader: nothing
# is compiled. All of its licences are free (LLVM Apache-2.0 with exceptions,
# mingw-w64 ZPL/public domain); no Microsoft SDK is involved.
#
# Use:  nix build --no-link --print-out-paths .#llvm-mingw
{ stdenv, autoPatchelfHook, zlib, zstd, src }:
stdenv.mkDerivation rec {
  pname = "llvm-mingw";
  version = "20261104";
  inherit src;
  # The flake input is already the unpacked upstream tree: no fetch, no unpack.
  dontUnpack = true;
  nativeBuildInputs = [ autoPatchelfHook ];
  # libstdc++ and libgcc_s; glibc is the stdenv's, libLLVM and libclang-cpp ship in lib/.
  buildInputs = [ stdenv.cc.cc.lib zlib zstd ];
  dontConfigure = true;
  dontBuild = true;
  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -r "$src"/. "$out"/
    # The flake input is a read-only store tree: its modes come along, so make
    # the copy writable before the removals below (Nix makes the output
    # read-only again at the end, exactly like the unpacked archive before).
    chmod -R u+w "$out"
    # The debugger is not part of any build and would pull libxml2, xz and ncurses in.
    rm -f "$out"/bin/lldb* "$out"/lib/liblldb*
    # Rust's standard library links `-lunwind`. With the import library next to libunwind.a
    # the linker takes the import library (rustc 1.99 does, 1.100 does not), and the
    # executable would then import libunwind.dll, which a clean Windows does not have.
    # Without it the one resolution is the static library, for every Rust version.
    rm -f "$out"/*-w64-mingw32/lib/libunwind.dll.a
    runHook postInstall
  '';
  # The archive holds Windows objects and import libraries next to the Linux tools.
  dontStrip = true;
  meta.description = "LLVM-based mingw-w64 toolchain (upstream release, patched for the Nix loader)";
}
