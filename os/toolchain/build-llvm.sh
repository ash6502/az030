#!/bin/sh
# Build the LLVM tools the az030 OS needs (llc and friends, M68k back end only),
# with the fixes in llvm-m68k-fixes.patch applied.
#
#   sh toolchain/build-llvm.sh VERSION TOOLCHAIN_DIR
#
# Needs a C++ compiler and Python 3; cmake and ninja are installed into a local
# virtualenv. Takes about an hour on a 4-core machine.
set -eu
VERSION=$1
TC=$2
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$TC/llvm-src/llvm-project-$VERSION.src
mkdir -p "$TC/llvm-dl" "$TC/llvm-src"
if [ ! -d "$SRC/llvm" ]; then
    TARBALL=llvm-project-$VERSION.src.tar.xz
    [ -f "$TC/llvm-dl/$TARBALL" ] || curl -sSfL -o "$TC/llvm-dl/$TARBALL" \
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-$VERSION/$TARBALL"
    tar -xJf "$TC/llvm-dl/$TARBALL" -C "$TC/llvm-src" \
        "llvm-project-$VERSION.src/llvm" "llvm-project-$VERSION.src/cmake" \
        "llvm-project-$VERSION.src/third-party" "llvm-project-$VERSION.src/libc"
    (cd "$SRC" && patch -p1 < "$HERE/llvm-m68k-fixes.patch")
fi
if [ ! -x "$TC/venv/bin/cmake" ]; then
    python3 -m venv "$TC/venv"
    "$TC/venv/bin/pip" install -q cmake ninja
fi
PATH=$TC/venv/bin:$PATH
cmake -G Ninja -S "$SRC/llvm" -B "$TC/llvm-build" -DCMAKE_BUILD_TYPE=Release \
    -DLLVM_TARGETS_TO_BUILD="" -DLLVM_EXPERIMENTAL_TARGETS_TO_BUILD=M68k \
    -DLLVM_DEFAULT_TARGET_TRIPLE=m68k-unknown-none-elf -DLLVM_ENABLE_ASSERTIONS=OFF \
    -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF \
    -DLLVM_INCLUDE_DOCS=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_ZLIB=OFF \
    -DLLVM_ENABLE_LIBXML2=OFF -DLLVM_ENABLE_LIBEDIT=OFF -DLLVM_ENABLE_LIBPFM=OFF \
    -DLLVM_ENABLE_BINDINGS=OFF
ninja -C "$TC/llvm-build" llc opt llvm-link llvm-dis llvm-extract
mkdir -p "$TC/llvm/bin"
for t in llc opt llvm-link llvm-dis llvm-extract; do cp "$TC/llvm-build/bin/$t" "$TC/llvm/bin/"; done
echo "llc installed in $TC/llvm/bin"
