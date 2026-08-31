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

On macOS the download is unsigned, so clear the quarantine flag once:

```sh
xattr -d com.apple.quarantine keal
```

On Windows, `keal build` needs a C compiler that is **not** MSVC — the
runtime checks arithmetic overflow with GCC and Clang builtins. Install
MinGW-w64 in a **POSIX-threads** build (the `win32` and `mcf` flavours
ship no `pthread.h`, which the actor scheduler needs). Everything else
works out of the box.

