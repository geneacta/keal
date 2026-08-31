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
for name in ${CC:-} cc gcc clang; do
    if "$name" --version >/dev/null 2>&1; then driver="$name"; break; fi
done
[ -n "$driver" ] || { echo "no C compiler after installing one"; exit 1; }

echo "keal will build with: $driver"
"$driver" --version | head -1
"$driver" -dumpmachine

"$driver" -dumpmachine | grep -q mingw \
    || { echo "$driver does not target mingw32; the runtime needs one that does"; exit 1; }
"$driver" -v 2>&1 | grep -q 'Thread model: posix' \
    || { echo "$driver has win32 threads; actor programs need the POSIX build"; exit 1; }
