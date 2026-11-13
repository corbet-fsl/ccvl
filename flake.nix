# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
#
# ccvl's Windows toolchain as Nix inputs. The llvm-mingw release archive (clang,
# lld and the mingw-w64 UCRT runtime for x86_64-pc-windows-gnullvm and
# aarch64-pc-windows-gnullvm) is a plain tarball input (flake = false): `nix flake
# archive` pushes its NAR to the srv Celler ci cache on every CI run (the existing
# archive step of the Forgejo workflows), so every runner substitutes it and a
# GitHub outage never blocks a Windows build. nixpkgs has no such package (and its
# clang-based mingw cross toolchains do not build at this pin), so this stays a
# fixed-output derivation of the upstream release until the flake-update frontier
# brings a nixpkgs that does. NOT a forge mirror: the source stays upstream.
#
# Pins: the nixpkgs input is the SAME revision the .ci jobs build their tools from
# (tool_source in .ci/run-cached.sh and .ci/release-build.sh). Bump both together.
# A new upstream llvm-mingw release is adopted deliberately: edit the
# llvm-mingw-dist URL below, then `nix flake lock --update-input llvm-mingw-dist`.
# No hand pin in shell, no new mechanism: the lock is the pin.
{
  description = "ccvl's Windows toolchain (llvm-mingw release, fixed-output, cached like every other toolchain)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/b7c2ada94fe99c15b0dbcf4d11fd7850b957a436";
    llvm-mingw-dist.url = "https://github.com/mstorsjo/llvm-mingw/releases/download/20261104/llvm-mingw-20261104-ucrt-ubuntu-22.04-x86_64.tar.xz";
    llvm-mingw-dist.flake = false;
  };

  outputs =
    { self, nixpkgs, llvm-mingw-dist }:
    {
      packages.x86_64-linux.llvm-mingw = nixpkgs.legacyPackages.x86_64-linux.callPackage ./.ci/llvm-mingw.nix {
        src = llvm-mingw-dist;
      };
      packages.x86_64-linux.default = self.packages.x86_64-linux.llvm-mingw;
    };
}
