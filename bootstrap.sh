#!/bin/sh
# Builds the official Keal compiler: the one written in Keal, compiled by
# itself, and proven to be at a fixed point before it is installed.
#
#   ./bootstrap.sh          ->  dist/kealc
#
# kealc turns a .keal program into a single self-contained C11 file:
#
#   dist/kealc program.keal > program.c && cc -O2 -std=c11 -o program program.c
#
# The Rust binary (`cargo build --release`) remains the full toolchain — VM,
# tree-walking interpreter, checker, REPL — and the oracle every stage of the
# self-hosted compiler is byte-for-byte verified against by the test suite.

set -e
cd "$(dirname "$0")"

echo "1/4 building the Rust toolchain (the oracle)..."
cargo build --release --quiet

echo "2/4 compiling the self-hosted compiler to native..."
./target/release/keal build selfhost/cbackend.keal > /dev/null

echo "3/4 verifying the fixed point..."
./cbackend selfhost/cbackend.keal > .bootstrap-check.c
./target/release/keal cgen selfhost/cbackend.keal | cmp -s - .bootstrap-check.c || {
    echo "FIXED POINT BROKEN: the bootstrapped compiler does not reproduce its own C" >&2
    rm -f .bootstrap-check.c cbackend.c cbackend.o cbackend
    exit 1
}
rm -f .bootstrap-check.c

echo "4/4 installing..."
mkdir -p dist
mv cbackend dist/kealc
rm -f cbackend.c cbackend.o

# MSYS treats `kealc` and `kealc.exe` as the same file, so the `mv` above
# lands the suffix on Windows without saying so. Announce what is there.
built=dist/kealc
[ -f "$built.exe" ] && built="$built.exe"
echo "$built — the Keal compiler, written in Keal, compiled by itself."
