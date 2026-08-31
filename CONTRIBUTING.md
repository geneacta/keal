# Contributing to Keal

Keal holds itself to a small set of non-negotiable rules; a change that
respects them is welcome from anyone. This file is the whole procedure.

## Build and verify

```sh
cargo build --release            # the Rust toolchain (the oracle)
cargo test --release             # the whole suite, native tests included
./bootstrap.sh                   # the self-hosted compiler, to its fixed point
```

A JDK and a C compiler unlock the interop tests; without them those
tests skip, they do not fail. `keal doctor` lists what this machine has,
next to the versions the interop suite was last verified against —
Keal pins *versions*, it does not vendor toolchains: compilers are
hundreds of platform-specific megabytes that every OS packages better
than a language repository can, and a differing version is not an
error, just a fact `cargo test --release` settles.

## The rules every change must respect

1. **Oracle and twin, byte for byte.** Every compiler-stage change lands
   in the Rust oracle (`src/lexer.rs`, `parser.rs`, `checker.rs`,
   `cbackend.rs`) *and* in the self-hosted twin (`selfhost/lexing.keal`,
   `parsing.keal`, `checking.keal`, `cbackend.keal`), and the four dump
   commands must agree byte-for-byte over the whole corpus:

   ```sh
   keal tokens f.keal   ↔   keal selfhost/lexer.keal f.keal
   keal ast f.keal      ↔   keal selfhost/parser.keal f.keal
   keal types f.keal    ↔   keal selfhost/checker.keal f.keal
   keal cgen f.keal     ↔   keal selfhost/cbackend.keal f.keal
   ```

   The suite runs this comparison over every `.keal` file in the
   repository — programs, examples, and the error corpora, where the
   *diagnostics* must match too, exit codes included.

2. **Three engines, one behavior.** The tree-walking evaluator is the
   specification; the bytecode VM and the native backend must match it
   exactly — output, panic messages, everything. When the C backend
   cannot compile something yet, it **refuses by name** with a clear
   message; it never mis-compiles. Runtime programs must be leak-free:
   `leaks --atExit` on the native binary says zero.

3. **Generated files regenerate, never edit.** After touching
   `src/prelude.keal` or `src/runtime.c`, rebuild (`cargo build
   --release`) and regenerate `selfhost/preludesrc.keal` /
   `selfhost/runtimesrc.keal` (each wraps the file in a raw-string
   function; the header comment in each says so).

4. **New behavior comes with tests.**
   * `tests/programs/` — self-checking programs (`assert`, silent, exit 0),
     run on both interpreters.
   * `tests/native/` — programs with printed output and an `.expected`
     snapshot, run on all three engines.
   * `tests/selfhost/parse-errors|type-errors|errors/` — programs that
     must be refused, named for what they prove.
   * `tests/fuzz/fuzz.py <keal> <count> [seed]` — the differential
     fuzzer; run a few thousand programs when you touch the checker.
   * `UPDATE_EXPECT=1 cargo test --release` rewrites snapshots.

5. **A test may skip because it cannot run, never because it would rather
   not.** Plenty of tests here stand down when a machine has no C
   compiler, no JDK, no git, no Python — the check genuinely cannot
   happen, and saying so is right. That is not the same as a test that
   *could* run and takes an easier road: the site's drift check linked
   what it needed and skipped where symlinks want elevation, so the one
   platform where the generator was broken was the one platform the test
   did not look at. It copies now. A test that skips on a platform is a
   test that platform does not have, and the bug it was written for will
   be found there first.

6. **Nothing decodes bytes without saying how.** Every `open()` and every
   `subprocess` call in `site/*.py` names `encoding="utf-8"`, and every
   write names `newline=""`. Python takes both from the machine's locale
   otherwise, and a machine whose codepage is cp1252 will mis-decode the
   compiler's own output into a page and exit 0 — the failure that does
   not fail. The same rule is why a diagnostic never embeds
   `std::io::Error`'s text: it is the operating system's sentence, in the
   operating system's language.

7. **Say it plainly.** Diagnostics explain and suggest (`-- note:` with
   the fix). Comments state constraints, not narration. Costs and limits
   go in the docs, not under the rug: see `docs/types.md` (the type
   rules), `docs/memory.md` (the memory model), `docs/drop.md`
   (unwinding and `deinit`), `docs/threads.md` (the actor plan),
   `docs/interop.md` (the boundary).

## Where things live

```
src/            the Rust oracle: lexer, parser, checker, interp, VM,
                cbackend, runtime.c, prelude.keal, tools (doc, bindgen,
                jbind)
selfhost/       the same compiler, written in Keal, held byte-identical
lib/            libraries beyond the prelude (the JVM gateway)
tests/          the corpora described above
examples/       runnable programs, including the interop demos
docs/           the design decisions, honestly stated
```

## Proposing a change

Fork, branch, keep the commit story readable (one concern per commit),
and make sure the three commands at the top are green before opening a
pull request. A PR that changes semantics should quote the rule in
`docs/` it follows — or start by proposing the doc change, which is
often the real discussion.
