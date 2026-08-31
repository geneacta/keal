# Releasing

A release of Keal is a git tag, a set of binaries, and a page that says
what changed. Nothing about the language depends on it — the repository
is always buildable from source — but a tag is what lets somebody run the
compiler without installing Rust first, and what lets a bug report say
*which* Keal.

## What a version means here

The project is pre-1.0, so the middle number carries the meaning:

* **0.x.0** — new language surface, or a change to what a program means.
  `weak`, actors on threads, `Any` natively, visibility and namespaces:
  each of those was a minor.
* **0.x.y** — fixes and internals with no visible change to a correct
  program.
* **1.0** will mean the semantics are frozen, which they are not: the
  cycle audit, typed exceptions and a module namespace are all still
  open, and each could change how a program is written.

## The release criteria

A tag is only cut when all four are green on the machine cutting it, and
the release workflow runs them again on every platform it builds for:

```sh
cargo test --release     # the whole suite, three engines, all corpora
./bootstrap.sh           # the self-hosted compiler, to its fixed point
python3 tests/fuzz/fuzz.py ./target/release/keal 3000
leaks --atExit -- ./some-native-binary   # macOS; zero leaks
```

Every test that reaches for a C compiler, a JDK, git or Python skips
itself when it is not there rather than failing — which is what lets the
same suite run on a machine that cannot compile C at all.

Plus the two rules that are not commands: `STATUS.md` describes the state
the tag is in, and every claim added to `README.md` or `docs/` since the
last tag is one the suite actually checks.

## Cutting one

1. **Bump the version** in `Cargo.toml`, and in the site's badge
   (`site/build.py`, the `v0.x.y` badge string) — then
   `python3 site/build.py`, which rewrites all 42 pages.
2. **Update `STATUS.md`**: what shipped, what is in flight, what is next.
3. Commit, as ever authored `Tony Renard <contact@geneacta.com>`.
4. **Tag and push:**

   ```sh
   git tag -a v0.7.0 -m "visibility, reserved words, namespaces"
   git push origin v0.7.0
   ```

5. The `release` workflow (see [`ci/README.md`](ci/README.md)) builds the
   compiler for macOS arm64, macOS x86_64 and Linux x86_64, runs the
   suite and the bootstrap on each, and opens a **draft** release with
   the three archives attached.
6. **Write the notes and publish.** GitHub's generated list of commits is
   the raw material, not the notes: say what changed for someone writing
   Keal, in the order that matters to them, and name what is still
   missing. The commit messages in this repository are written to make
   that easy.

## What ships in an archive

`keal` (the compiler and the runner), `README.md` and `LICENSE`. That is
all a user needs: the prelude and the C runtime are compiled into the
binary, so there is nothing to install beside it. A C compiler is only
needed for `keal build`, and `keal doctor` will say whether one is there.

Four platforms are built and tested, and held to the same standard:
macOS on Apple silicon, macOS on Intel, Linux x86_64 and Windows x86_64.
Windows has been run on both of its ABIs — `x86_64-pc-windows-gnu` and the
`x86_64-pc-windows-msvc` the workflow ships — with the same result: the
whole suite, the bootstrap to its fixed point, and JNI from an actor
thread. The only test that stands down there is the ThreadSanitizer one,
because MinGW has no ThreadSanitizer.

**Upgrading an existing Windows checkout.** `.gitattributes` renormalises
only the files a commit rewrites, so a `git pull` onto a tree checked out
before it existed leaves the rest at CRLF — a half-converted tree that
fails a few tests confusingly. Say it in the notes: after pulling, run
`git rm --cached -r . ; git reset --hard`, or re-clone.

**What a Windows user needs, and why.** The compiler itself needs nothing
but Rust. `keal build` needs a C driver, and not any of them: the runtime
checks arithmetic overflow with the GCC and Clang builtins, so **MSVC
cannot compile it** — which matters because MSVC is exactly what a default
Rust install on Windows brings. Install **MinGW-w64**, and a
**POSIX-threads build** of it: the actor scheduler wants `pthread.h`, and
the `win32` and `mcf` flavours do not have one. LLVM clang works if it
targets mingw32. `keal doctor` reports which driver it found, and a build
that finds only `cl.exe` says so by name rather than reporting no compiler
at all.

## What is not automated, and why

**Publishing to crates.io** — the crate is the compiler, not a library
anyone depends on, so it would carry a promise about API stability the
project cannot yet keep. `cargo install --git` works today.

**Homebrew, apt, winget** — worth doing when there is a 1.0 to package.
A formula that installs a moving pre-release ages badly.

**Signing and notarisation** — macOS will quarantine an unsigned
download, and users will have to clear it by hand
(`xattr -d com.apple.quarantine keal`). Fixing that needs an Apple
Developer account; it is on the list, and it is honest to say the
download is unsigned until then.
