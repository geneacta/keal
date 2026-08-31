#!/bin/sh
# Puts a C compiler on a Windows runner, and proves it is the right one.
#
# A Windows image has no `cc`, and the compiler this runtime needs is not
# MSVC: overflow is checked with GCC and Clang builtins. Without this every
# test that compiles C skips itself, and a green Windows leg means far less
# than it appears to.
#
# It lives in the repository rather than inside the workflow so that this
# logic can be changed without pasting a workflow file through a web UI.

set -e

# Chocolatey's `mingw` is GCC for x86_64-w64-mingw32 with POSIX threads,
# which the actor scheduler needs. It no-ops when already installed, so
# there is nothing to guess about the image.
choco install mingw --no-progress -y
MINGW_BIN='C:\ProgramData\mingw64\mingw64\bin'
[ -n "$GITHUB_PATH" ] && echo "$MINGW_BIN" >> "$GITHUB_PATH"
PATH="/c/ProgramData/mingw64/mingw64/bin:$PATH"
export PATH

# Assert on the driver `keal build` will actually pick, not on `gcc`
# specifically: `c_driver()` walks CC, then cc, gcc, clang, and takes the
# first that answers. A machine whose `cc` is not its `gcc` — a wrapper, a
# shim, a stale symlink — would otherwise pass a check on one and build
# with the other.
driver=""
if [ -n "${CC:-}" ]; then
    # `CC` is a whole command and not one more bare name: `CC="zig cc"` is
    # what a Windows machine reaches for, having no `cc`, and the compiler
    # splits it into program plus arguments. The word splitting below is
    # deliberate, and it is why `CC` cannot share the loop.
    driver="$CC"
else
    for name in cc gcc clang; do
        if "$name" --version >/dev/null 2>&1; then driver="$name"; break; fi
    done
fi
[ -n "$driver" ] || { echo "no C compiler after installing one"; exit 1; }

echo "keal will build with: $driver"
$driver --version | head -1

# What it must NOT be, rather than what it must say. MSVC cannot compile
# this runtime, and a target is spelled `x86_64-w64-mingw32` by GCC and
# `x86_64-windows-gnu` by clang and zig — all three of which work.
target=$($driver -dumpmachine 2>/dev/null || echo unknown)
echo "target: $target"
case "$target" in
    *mingw* | *windows-gnu*) ;;
    *) echo "$driver targets $target; the runtime needs a GNU-ABI Windows target"; exit 1 ;;
esac

# Likewise: reject the flavour that is known to break rather than require
# one spelling of the one that works. A `win32` MinGW has no `pthread.h`,
# and every actor program needs one.
if $driver -v 2>&1 | grep -q 'Thread model: win32'; then
    echo "$driver has win32 threads; actor programs need the POSIX build"
    exit 1
fi
