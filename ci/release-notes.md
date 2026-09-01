<!-- The footer of every release. What a given release contains comes
     from the tag's own message, which the workflow puts above this. -->

## Installing

Keal is a statically typed, self-hosting language: three engines — a
tree-walking interpreter, a bytecode VM, and native code through C11 —
that must print the same bytes for every program in the test suite.

**Download** the archive for your platform below. Each contains one
`keal` binary, the README and the licence; the prelude and the C runtime
are compiled into the binary, so there is nothing to install beside it. A
C compiler is only needed for `keal build`, and `keal doctor` reports
whether one is there.

The Linux build needs **glibc 2.34 or newer** — RHEL 9, Ubuntu 22.04, Debian
12 and anything later. Ubuntu 20.04 and Debian 11 are 2.31 and will not load
it. The floor is set by `pthread_create` and its neighbours, which the actor
runtime needs; nothing exotic pins it.

That number is worth stating carefully, because the tidy way of finding it is
wrong. The highest `GLIBC_` string in the binary is 2.39, and reading that as
the requirement would put the floor five minor versions too high and rule out
distributions that run it perfectly well. The two 2.39 symbols are WEAK: the
loader resolves them to null when they are absent and the program starts
anyway, because they are an optional fast path with a fallback behind them.
Only the strong symbols set a floor.

On macOS the download is unsigned, so clear the quarantine flag once:

```sh
xattr -d com.apple.quarantine keal
```

On Windows, `keal build` needs a C compiler that is **not** MSVC — the
runtime checks arithmetic overflow with GCC and Clang builtins. Install
MinGW-w64 in a **POSIX-threads** build (the `win32` and `mcf` flavours
ship no `pthread.h`, which the actor scheduler needs). Everything else
works out of the box.

