# Releasing

A release of Keal is a git tag, a set of binaries, and a page that says
what changed. Nothing about the language depends on it — the repository
is always buildable from source — but a tag is what lets somebody run the
compiler without installing Rust first, and what lets a bug report say
*which* Keal.

## What a version means here

From 1.0, the first number carries the promise and the second carries the
work:

* **1.x.0** — new surface that takes nothing away. A program that compiles
  today compiles tomorrow and means the same thing.
* **1.x.y** — fixes and internals with no visible change to a correct
  program.
* **2.0** would mean something a working program has to be rewritten for.

### 1.2.0 broke that, deliberately, and this says so

1.2.0 takes three things away. `Ord.compareTo` answers a `Comp` instead of
an `Int`, `Comp` carries no methods where it carried five, and `less`,
`equal` and `greater` became reserved words. By the rule above that is a
2.0, and it was released as 1.2.0 anyway. Tony's call, made with the
breaks in front of him, and the reasoning is worth recording rather than
leaving as an inconsistency for somebody to find:

**The promise had nobody to keep it to yet.** The two programs written in
this language are a GUI framework and a web framework, both in-house, both
migrated the same week by the sessions that wrote them — the whole cost of
the change was three files and an afternoon. A major number spent before
anyone outside is affected buys nothing and spends the one signal that
tells a stranger to read carefully.

**What protects a reader is the notes, not the number.** The release names
what breaks in its first section, with the line to write instead for each.
That is the part that has to be right.

**The rule stands for what comes next.** It is not suspended and it does
not become a habit: the day there is code out there that this project did
not write, the first number starts meaning what it says here. If a second
release takes something away under a `1.x` number, this section stops
being a recorded decision and becomes a pattern, and the honest thing then
is to change the rule rather than the exception.

### What 1.0 freezes, and what it does not

**Frozen: the language.** Everything `keal check` accepts today it will keep
accepting, with the same meaning. That is a real promise and it was tested
before it was made: every addition still on the list — enum variants that
carry data, generic traits, extension methods, sized integers, and each of
the seven held words — is **refused today**, so adding it can only make an
invalid program valid. `async`, `await`, `yield`, `sealed`, `super`,
`static` and `typealias` are reserved for exactly that reason.

That reasoning is why 1.2.0 is the exception recorded above and not a
second way of reading the rule. It did not add something refused today; it
changed the meaning of something accepted, three times. The three words it
reserved were not held in advance, which is exactly what holding words is
for — and that is the lesson to take rather than the licence: a word the
language may want later should be reserved before anyone can name a
variable with it, not after.

**Not frozen: the standard library.** It grows, and it grows by addition.
Seventy-seven built-in methods and a prelude is a small library and the
project says so; the next things in it should come from programs somebody
actually writes.

**Not frozen, and never was: the C the backend emits.** It is an
implementation detail with one obligation — that the program behaves as the
interpreters do — and the suite is what holds it there.

**What 1.0 does not claim.** Not a large ecosystem, not a battle-tested
standard library, not a crowd. It claims that the language is finished
enough to build on without the ground moving.

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
   git tag -a v1.0.0 -m "the language is finished enough to build on

   Kotlin's shape over a C-family syntax ...
   "
   git push origin v1.0.0
   ```

   The message is not a label: it becomes the top of the release notes, so
   write it as the paragraphs a reader wants — what changed, for whom, and
   what is still missing.

5. The `release` workflow (see [`ci/README.md`](ci/README.md)) builds the
   compiler for macOS arm64, macOS x86_64, Linux x86_64 and Windows,
   runs the suite and the bootstrap on each, and **publishes** the release
   with the archives attached.
   It publishes rather than drafts because the four criteria above were
   green on the machine that cut the tag and are green again on every
   platform the workflow builds for — there is nothing left to read over
   first. The notes are already written: the tag's own message at the top,
   which is why step 4 asks for a real one rather than a version number,
   then the permanent installing section from `ci/release-notes.md`, then
   GitHub's generated commit list under both as raw material.

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
