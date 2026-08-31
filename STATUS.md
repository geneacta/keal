# STATUS — where the work stands, and how to resume it

*Updated: 2026-08-31 (version 0.8.0). This file is the hand-off: if a session dies, the next
one reads this and continues without archaeology. Keep it current at every
commit that leaves work in flight.*

## The iron rules (never break these)

1. **Author**: commits are authored `Tony Renard <contact@geneacta.com>`
   only — never add a co-author line.
2. **Oracle + twin, byte for byte.** Every compiler-stage change lands in
   the Rust oracle first (`src/`), is mirrored in the self-hosted twin
   (`selfhost/`), and the four dump commands must agree byte-for-byte over
   the whole corpus: `keal tokens` ↔ `selfhost/lexer.keal`, `keal ast` ↔
   `selfhost/parser.keal`, `keal types` ↔ `selfhost/checker.keal`,
   `keal cgen` ↔ `selfhost/cbackend.keal`.
3. **Generated embeds**: after editing `src/prelude.keal` or
   `src/runtime.c`, regenerate `selfhost/preludesrc.keal` /
   `selfhost/runtimesrc.keal` (python one-liners wrapping the file in a
   raw-string `package fun ...Source(): String` — `package`, since
   visibility landed and the twin's files read them from beside them),
   **after** `cargo build --release` (the binary embeds them via
   `include_str!`).
4. **Match or refuse**: the native backend never mis-compiles — it refuses
   by name. Runtime behavior must match the interpreters (messages
   included); `tests/native/*` runs on all three engines.
5. **Verification loop** (run before every commit):
   - the four-corpora loop (see below), `cargo test --release` (currently
     36 green incl. bootstrap fixed point, actor TSan, actor-thread JNI,
     the site tour's printed outputs), `./bootstrap.sh`.
   - corpora loop, for each cmd/driver pair above:
     `for f in tests/programs/*.keal examples/*.keal tests/native/*.keal
     tests/native-extern/*.keal tests/selfhost/*.keal selfhost/*.keal
     src/prelude.keal lib/jvm.keal tests/selfhost/errors/*.keal
     tests/selfhost/parse-errors/*.keal tests/selfhost/type-errors/*.keal`
     → oracle output == twin output, exit codes too.
6. Keal gotchas when writing twin code: no narrowing on `var` (copy to a
   `val` first), no leading `+`/`and`/`or` at line starts (ASI), no
   turbofish (`val x: Box<Int> = Box(...)`), bare tuples need parens in
   `when` arm bodies.

## DONE (all pushed to geneacta/keal, author Tony Renard)

- Full language: generics (monomorphized), traits, records, null safety,
  smart casts, `when`, tuples, destructuring, lambdas/closures, modules,
  flat-precedence logical operators, `fun`/`proc`, `if`/`unless`.
- Three engines agreeing byte-for-byte: tree-walker (spec), bytecode VM
  (default), native via C (`keal build`). Memory model doc + `keal layout`.
- **Self-hosting complete**: lexer, parser, checker, C emitter written in
  Keal, each held to its oracle; `./bootstrap.sh` → `dist/kealc` compiles
  itself to a **fixed point** (suite-verified: `the_compiler_compiles_itself`).
- Lazy sequences + deterministic actor model in the prelude (pure Keal).
- Interop: `borrow`/`own` String, by-value records (`Keal_X` mirror structs
  with `KEAL_MIRROR_X` guards), `keal emit-header`, link inputs
  (`.a`/`-l`/`-L`/`-I`/`-D`), `keal bindgen` (C headers → extern decls,
  skip-with-reason), verified Rust demo (`examples/interop/rust/`), JVM
  gateway `lib/jvm.keal` + verified `java.time` demo
  (`examples/interop/java/`). Plan in `docs/interop.md`.

## IN FLIGHT

**TYPED EXCEPTIONS — DONE, all three engines.**
`throw` carries ANY value now (a function is the one refusal: a signature
has no run-time identity to catch it by). `try` takes a LIST of clauses,
tried in order: `catch (e: T)` binds the value whole when its type
matches, `catch (e)` binds the MESSAGE and must come last (a clause behind
it is refused as unreachable). No match rethrows unchanged. Built-in
failures throw their own message, so `catch (e: String)` catches an
overflow.
MECHANISM: `RtError` gained `value: Option<Value>` — built-ins fill it
with their message, so there is one path and not two. The VM lands on its
handler with the VALUE pushed (not the message) and the clauses compile to
`Dup; IsType; JumpIfFalse` chains, the catch-all to `Interpolate(1)`, and
a fall-through to `Throw` — no new opcodes. `Op::Throw` takes any value.
The tree-walker uses `type_matches`, the same test `is` uses.
NATIVE: the C unwind grew a value beside its message buffer — a `KealAny`,
tag and payload, the machinery that already existed. `keal_throw_value`
adopts the value on a fresh unwind (one already unwinding keeps the first,
the rule the message already followed) and takes its message from
`keal_any_display`, which is where the interpreters take theirs, so the
text is the same by construction. `keal_unwind_is(ti)` is the clause test;
a message-only unwind — every built-in failure — answers to `String`,
which is what it is, so no built-in needed a second path. Two takers:
`keal_unwind_value_take` hands the value over owned, `keal_unwind_take`
hands the message over and RELEASES a value the clause did not ask for.
A `throw` of a String still emits `keal_panic` unchanged, so the whole
existing corpus's C output is byte-identical — only new programs move.
The clauses emit as a chain of `if`s, each jumping past those under it,
and when the last one names a type a `check_unwind` under the chain lets
an unmatched value go on unwinding. 0 leaks on `tests/native/trycatch`.
TWIN: `CatchN`, clause parsing, `pTry`, the checker's clause loop, and —
the bug this found — the LOADER NEVER STAMPED a try's handler with its
file. Harmless while a handler had no type to resolve; the moment a clause
carried one it resolved against `<prelude>`, which declares none of a
program's classes.
Corpus: `tests/programs/exceptions.keal` grew the typed cases (record,
Int, String, an unmatched clause rethrowing to an outer one, and a
built-in overflow caught as a String); `throw-not-a-string.keal` is gone
because `throw 42` is legal now, replaced by `throw-a-function.keal` and
`catch-after-catch-all.keal`.

**WINDOWS IS DONE, on both ABIs.** 36/36 on
`x86_64-pc-windows-gnu` and 36/36 on the `x86_64-pc-windows-msvc` the
release workflow ships — compiler, both interpreters, native builds,
actors, JNI from an actor thread, the bootstrap to its fixed point, the
self-hosted twins byte-identical, the audits agreeing three ways, and the
site generator. One skip: TSan, which MinGW does not have.
The MSVC run proves the load-bearing claim: an MSVC-ABI `keal.exe`
shelling out to MinGW `gcc` mixes nothing, because the emitted program is
its own executable sharing no runtime with the compiler. Three toolchains
in one process tree in `jvm_calls_work_from_actor_threads` — MSVC-built
compiler, MinGW gcc, MSVC-built JDK — and none notices the others.
WHAT WINDOWS COST, and it is the list worth keeping: CRLF in two halves
(text-mode stdout AND `.gitattributes`), `\r\r\n` CORRUPTION of content
that already had CRLF, path separators in diagnostics, `io::Error`'s
LOCALISED text in a compared message, `keal jbind` emitting unparseable
Keal from a `C:\` path, a hardcoded `darwin` in the JNI include path, the
JNI link line and the PATH a JNI binary needs, `cc` not existing there,
`.exe`, five JVM tests with no C-compiler guard, Python's `open()` at
cp1252 DELETING a page, `subprocess` at cp1252 writing mojibake and
exiting 0, and a drift test that skipped on the one platform its subject
was broken on.
THE PATTERN: four times, Windows used a locale-specific default where the
code assumed UTF-8, and the machine was right each time. And four defects
were really a missing or a skipped test.
Toolchain a Windows developer needs: MinGW-w64 POSIX-threads (NOT win32
or mcf — no `pthread.h`), because MSVC cannot compile the runtime's
overflow builtins. Everything except MSVC Build Tools installs per-user
with no elevation.

**A claim I made and withdrew: the prelude does NOT leak.** The audit
reported `3 Score` and `3 Sequence` surviving `tests/programs/sequences.
keal` on all three engines, and I read that as a cycle in the standard
library — `Sequence` holding the closure that makes its iterator. It is
not. Those are TOP-LEVEL BINDINGS, and a global lives to the end of the
program by design on every engine; the audit reports it because it is
true. `val scored = seq([...])` alone reports `2 Score, 1 Sequence`, which
is the whole story. README and memory.md are corrected.
What is real, and small: on that program the VM keeps two `Sequence`s and
the tree-walker five `SeqIter`s that a native build does not. No behaviour
depends on either and no `deinit` is missed.
A GUESS, NOT A FINDING, and it is written here only so nobody re-derives
it: the residues are asymmetric — different types, different counts, one
of them a type neither other engine reports at all — which SUGGESTS two
retentions rather than one at two sizes. Two data points and one program.
Whoever picks this up should measure before believing it, including the
part where it came from a peer session and I agreed with it; being
written down is not evidence.
Fixed on the way: a lambda captured the receiver whether it named `this`
or not, which is why the prelude's care in lifting `val makeIter =
this.iterFn` out of its closure bought nothing. `this` is a capture like
any other in `collect_free` now; the tree-walker's report on that program
fell from 19 objects to 11.

**A `deinit` the tree-walker never ran — found by the audit, half fixed.**
The audit's first real use found the engines disagreeing about what a
program left behind, and behind that a genuine divergence: the
TREE-WALKER RETAINS WHERE THE OTHER TWO DO NOT, because its closures
capture the whole enclosing `Scope` while the VM and the C backend capture
the values they need. Two shapes, one fixed and one not:
FIXED — a closure bound in the very scope it captured (`val f = { ... }`
in a body, the common case). Scope holds closure, closure holds scope,
count never reaches zero, nothing in that scope is ever released and its
`deinit` never runs. `Scope::close` breaks that self-edge when a scope is
finished with (from the three sites that finish one: `exec_block`, a loop
turn, a call frame) — and it is STRICT about when it may: every closure
over the scope must be held by that scope alone AND nothing else may hold
the scope. The first attempt was not, and it broke the prelude's
sequences: an escaped iterator closure still reads `advance` through the
scope chain, so a scope anything escaped from must be left exactly as it
was. `Scope::empty` also lets the globals go when a program ends, which
is what makes a top-level object's `deinit` run at all.
NOT FIXED — an escaped closure stored in an object the same scope holds
(`val h = Holder({ -> t.n })`). Object holds closure, closure holds scope,
scope holds object. The VM has no such cycle because its closure holds
values, not the frame. Repro in this session:
`class Thing { deinit }` + `class Holder(val make: () -> Int)`; the VM and
native print `thing 1 died`, the tree-walker does not. THE FIX is to make
the interpreter capture free variables by value, with cells for the
mutable ones, exactly as the VM does — `cbackend::collect_free` already
computes the set, and `copyClosure` already does the rebuild for actors.
Substantial, and the reference engine is the one that is wrong.
Regression test for the fixed half: the closure-capture block at the end
of `tests/programs/deinit.keal`, green on all three engines.

**The cycle audit, on the interpreters. DONE; the native half is next.**
`KEAL_AUDIT=1 keal run prog.keal` prints, at exit, what outlived the
program by type — the evidence `docs/memory.md` §5 promised instead of a
collector. `src/value.rs` gained a `pub mod audit`: a thread-local
`HashMap<String, i64>`, `born`/`died` called from `Instance::new` and the
`Drop` impl, and `report()` from `main.rs` after the program's values are
gone. `wanted()` reads the environment ONCE through a `OnceLock`, so the
counts cannot be made to lie mid-run, and a program that does not ask
pays one boolean read per object — output byte-identical.
THE ONE SUBTLETY: the drop hook's copy in `value.rs` is a MOVE, not a
birth, and the original is dying as it is made — so it calls `born`
explicitly to cancel the `died` that follows. Every other instance now
comes from `Instance::new` (interp, VM, and `native.rs`'s deep copy).
Report goes to stderr, sorted by class, with the note that names `weak`.
Tests: `tests/audit/cycle.keal` (a cycle beside an acyclic pair whose
deinits do run) + `the_audit_names_what_outlived_the_program`, which also
asserts the audit stays silent when unasked. Suite 34/34, corpora
656/656.
DONE natively too: `keal build --audit` defines `KEAL_AUDIT`, and the
runtime carries a 256-row registry keyed by class name (`keal_audit_born`
/`_died`/`_report`) whose report is the interpreters' words, in the
interpreters' order, on stderr. Rows go behind `keal_actor_lock` when
KEAL_ACTORS is on, so threads count one total. Emission: `born` after the
`keal_alloc` in `X_new`, `died` in `X_release` AFTER the drop-hook block
(an object that queues its `deinit` returns once and dies once), and
`keal_audit_report()` before main's `return 0`. Counted under the CLASS's
name, not the struct's, so a generic's instantiations count as one class
— which is what the interpreters count. `--audit` is a build flag, not an
environment variable, because a binary cannot grow counters after it is
compiled; it is stripped in `main.rs`'s arg pass like `--ast`/`--vm`, so
it may sit anywhere on the line. `keal emit-c` takes it too, which is how
oracle and twin are compared with it on; `keal cgen` never does, so the
corpora stay pure. Twin mirrored (`auditMode`, same three emission
points, `--audit` in its driver). Test:
`the_native_audit_says_what_the_interpreters_say` builds
`tests/audit/cycle.keal` twice and demands the audited binary's stderr
equal both interpreters' byte for byte, and the unaudited one say
nothing.

**Dependencies, step one: `keal.toml` + `keal fetch` + `dep:` imports.
DONE.** A project's manifest names it and lists what it depends on, each
a git repository at an exact `tag` or `rev`; `keal fetch` (Rust-only
tool, like bindgen/jbind/doc — `src/fetch.rs`, manifest reader in
`src/manifest.rs`, a TOML subset with no crate behind it) clones into
`.keal/deps/<name>/` and checks the commit out detached. A program writes
`import "dep:geometry/shapes.keal"`, which the LOADER resolves to
`.keal/deps/...` beside the nearest `keal.toml` (walking up from the
importing file). THE SPLIT THAT MAKES THIS WORK: only `keal fetch`
touches the network — the compiler reads what is on disk — so a project
may commit `.keal/deps/` and build with no git, and the twin needs no
notion of fetching (it mirrors `resolveImport`/`projectRoot` only).
A missing dependency says `run keal fetch`, in both compilers.
Tests: `tests/deps/` (a COMMITTED dependency, three engines, plus
`missing.keal` for the message) and `fetch_puts_a_dependency_where_an_
import_finds_it`, which makes a git repo on the spot, tags it, fetches
and runs — skipped where git is absent. Suite 33/33, corpora 656/656,
bootstrap fixed point.
TRANSITIVITY — DONE. `keal fetch` reads a dependency's own `keal.toml`
and fetches what it asks into the SAME `.keal/deps` (flat, not nested:
two copies of a library are two sets of types and a program holding both
could not say which it meant). Flat means two askers can disagree, and
nothing can reconcile a pinned commit against another pinned commit — so
it names both and stops, which is a worse message than a resolver's and a
more honest one. `keal.lock` records the COMMIT each name resolved to
(not the tag, which can be moved) and who asked for it.
THE RULE THAT MAKES FLAT WORK: a `dep:` import resolves against the
OUTERMOST `keal.toml` above the file, not the nearest — otherwise a
library's own `dep:` imports would look inside the library. `root_of` in
`src/manifest.rs`, `projectRoot` in the twin's loader, mirrored.
Test: `a_dependency_may_have_dependencies` builds a three-repo chain on
the spot (app -> mid -> deep), fetches, runs it on both engines, checks
the lockfile names the asker, then makes the two-version clash and
demands both askers appear in the error. Skipped without git.

**NAMESPACES — DONE. Two files may declare the same name.** One pass
(`plan_namespaces` / `planNamespaces`) runs before anything is checked: it
gives every top-level declaration a UNIQUE NAME (the source name for the
first claimant, `parse#2` for the next — `#` is unwritable in Keal, so a
minted name can never be one a program chose; the C backends `flatten` it
to `_dup2`, and the pass skips a spelling whose flattened form any file
declares), and records what each file can see: itself, everything its
UNALIASED imports reach, then the prelude (loaded, not imported).
`import "./x.keal" as x` puts nothing in the bare set; `x.parse` and
`x.Node` are rewritten to the unique name by `unqualify` (expressions)
and `global_key` (types, which the parser now accepts dotted). A name
two visible modules declare is an error WHERE IT IS WRITTEN, once
(`ambiguous_at` dedupes: a callee resolves twice, as a name and as a
call), naming both files — never at the import.
THE SUBTLETY THAT COST THE MOST: only LOCALS shadow. `lookup` finds
globals in scope 0, so the resolution has to run whenever `lookup_local`
finds nothing — checking `lookup` instead silently kept the first
declaration and no ambiguity was ever reported.
Where nothing collides the unique name IS the source name, so the whole
corpus is unchanged. Twin mirrored, including two Keal-side workarounds:
a `Map<_, Int>` read needs `?: -1` (the C backend cannot compile `m[k]`
to an `Int?` yet), and the twin's AST node names had to become `var`.
Tests: `tests/programs/namespaces/` (three engines, two modules with the
same `parse` and `Node`, one aliased, incl. a `config.Node` annotation)
+ suite test `namespaces_keep_two_modules_apart`;
`tests/selfhost/type-errors/ambiguous-name.keal` + its `namespace-lib/`.
Verified: corpora 4x164 = 656/656, suite 30/30, bootstrap fixed point,
fuzz 3000 clean, 0 leaks. Docs: `docs/packages.md` rewritten as-built
(and it argues the package-manager order: manifest of git URLs, then a
lockfile, registry last if ever), language.md §13, TUTORIAL §10, README.

**Visibility — DONE IN FULL (stages A and B), five words reserved.**
Stage B: a class's members take the same modifier with the same default
(a member that says nothing is private to the file declaring the class);
a RECORD is the exception that proves it — its fields follow the record's
own visibility unless one says otherwise, because a record is its fields
(`member_vis` / `memberVis`). A method answering a trait the class
implements is always reachable, or `a + b` (which is `a.plus(b)`) would
depend on where it is written. Enforced at field read, field assignment,
method call, and a method taken as a value.
RESERVED WORDS: `public`, `private`, `package`, `internal`, `protected`
are real keywords now, not contextual ones; `internal`/`protected` name
no rule and are refused where written. `Vis::Unset` distinguishes
"nothing written" from an explicit `private` — the record rule is
unsayable without it. `record`, `trait`, `weak`, `native` and `extern`
stay contextual; both lists are in language.md §1 now.
MIGRATION of the members, by the same compiler-driven loop: of ~2700
members in `selfhost/`, **121** had to open; all 1391 of cbackend's
stayed private, and `ast.keal` is the only file open all the way through.
Two migrator bugs cost time and are worth remembering: it skipped lines
whose class already carried a modifier, and it truncated a member line to
its matched prefix — the repair was positional, against `git show HEAD:`.
(The namespace half that used to be listed here as next is DONE — see
the entry above.)

**Visibility, stage A — top-level declarations.** A declaration that
says nothing is **private to its own file**; `package` opens it to every
file in the same directory — a package IS a directory, nothing declares
one — and `public` opens it to whoever imports the file. The three words
are contextual, like `record` and `weak`, so no program that used one as
a name broke. Written on a top-level `fun`/`proc`/`class`/`record`/
`trait`/`extern fun`/`val`/`var`; inside a body there is nothing to write
it on, and `public` before anything else is refused by name.
MECHANISM: `Vis` on the AST (oracle `src/ast.rs`, twin `var vis: String`
where "" is private); the parser reads the modifier before the
declaration keyword, so a declaration's span still starts at its keyword
(that alignment cost one corpus divergence on ctor params); astdump
prints the modifier only when it is not private, so every un-annotated
program dumps exactly as before. The checker learns each file's package
from `Sources` (`learn_packages`, called at ALL FOUR entry points — the
three `Checker::new()` sites in main.rs plus `check()`; missing three of
them is why the first bindgen run said "another file"), records vis+home
on Binding/ClassInfo/TraitInfo, and refuses at three points: a name
written as a value, a class named to construct one, and every class or
trait named inside a written type (`check_type_names`, which must skip
names that are type parameters in scope — the prelude's `Tuple3<A, B, C>`
otherwise collided with a user class named `C`).
MIGRATION: driven by the compiler itself
(`scratchpad/migrate.py`): check, read back every "is private to" it
reports, mark exactly those `package` (all readers in the same
directory) or `public`, repeat to a fixed point. Result worth keeping:
of ~250 declarations in `selfhost/`, only ~60 needed opening —
`cbackend.keal`'s 65 are ALL private. Prelude and `lib/jvm.keal` are
public throughout (a standard library is its public surface); `keal
jbind` and `keal bindgen` now GENERATE `public` declarations, since a
generated module exists to be imported.
Tests: `tests/selfhost/type-errors/visibility.keal` (+ its
`visibility-lib/`) covers private, package-across-directories and a
private class named as a type; the modules program test now reads
`package class Vec2`. Verified: corpora 4x162 = 648/648, suite 29/29,
bootstrap fixed point, fuzz 3000 clean, the site tour still prints what
it promises. Docs: language.md §13 rewritten as "Modules and
visibility", TUTORIAL §10, README "What remains".
STAGE B, still to do: the same on class members — `public val n`,
private-by-default fields and methods. The parser and both AST already
carry `vis` on `CtorParam`/`FieldDecl`/methods and the dumps print it;
what is missing is the checker refusing a hidden member at a field
access or a method call, and the migration of every record whose fields
are read from another file. Note the consequence Tony should weigh: a
`record` is its fields, so every record used across a file boundary will
need `public` on each one unless records are given the rule that their
fields follow the record's own visibility.

Nothing — operators/Comp/guarded-return, `keal jbind`, the loader sugar
`import java.time.LocalDate`, the verified Go demo, exceptions
(`throw` / `try`-`catch` on all three engines incl. native checked
unwinding), the six-language polyglot demo, the ternary family AND
`weak` are **DONE and pushed**. See NEXT.

**The site, the editors, the release machinery — and two bugs the site
found. Version 0.6.0.** `site/build.py` generates all 42 pages from one
place, in English and in French with nothing said in one language that is
not said in the other: landing, tour (12 chapters), the `docs/*.md`
reference documents through a small markdown converter, one "coming from
X" page for each of ten languages (`site/coming.py`), and the standard
library re-dressed from `keal doc`'s own output. `site/content.py` holds
the authored prose. `editors/README.md` says how to install the VS Code
extension (a symlink into `~/.vscode/extensions`, or a `.vsix`), how
JetBrains reads the same folder as a TextMate bundle, and what a
TextMate grammar cannot do (structural editing — that wants a language
server, still unwritten); the extension is 0.2.0, Apache-2.0, and its
grammar knows `weak` and `deinit`. `ci/` holds the two workflows the
tooling cannot push itself (no `workflow` scope): `pages.yml`, installed,
and `release.yml`, not yet. `RELEASING.md` states what a version means
here, the four criteria a tag must meet, and what is deliberately not
automated (crates.io, Homebrew, signing).

TWO REAL BUGS, both found by running the tour's twelve snippets on all
three engines rather than trusting the page that says they are checked:
1. **A widened literal kept its old type.** `val r: Float = 1 / 2` gave
   0.5 on both interpreters and **0.0 natively** — a silent
   mis-compilation, the one thing the backend promises never to do.
   `widen` rewrote the `Int` leaves into `Float` nodes but left
   `e.ty = Int` on them, so the C backend emitted `keal_div(1.0, 2.0, 1)`
   into an `int64_t`. Fixed in `widen` (oracle `src/checker.rs`, twin
   `selfhost/checking.keal`): every node it touches now records `Float`.
   The `keal types` dumps change on both sides identically.
2. **`?.` to a value member did not compile at all.** `b?.n` with
   `n: Int` emitted `KealOptI64 _t = NULL;` — a `cc` error, not a refusal
   by name; and `s?.length` emitted `_t->k_length` on a `KealStr`,
   because the built-in property was looked up under the receiver's
   *nullable* type. Fixed in `cbackend` `guarded()` (the absent value is
   `opt_null(inner)`, the present one `opt_wrap`) and `field()` (the
   property is looked up under `non_null()`, and goes through the guard).
   Twin mirrored byte-for-byte.
   Tests: `tests/native/safe-chain.keal` + `.expected` (three engines:
   `?.` to Int/Float/Bool/String fields and methods, built-in `length`
   and `size`, `?:` after each, and the widened literals), plus asserts
   in `tests/programs/numbers.keal` and `nullability.keal`.
Also fixed: `docs/language.md` had a paragraph inserted inside the type
table, which cut the `Nothing` row off its header AND sent
`site/build.py` into an infinite loop (a line starting with `|` that
opens no table matched no branch and never advanced). The document is
repaired and the converter now consumes such a line as text.
The tour's own promise is now checked rather than asserted: it says every
snippet is a real program printing exactly what is beside it, so
`site/checktour.py` runs all twelve on both interpreters — and natively
wherever the backend accepts them — against the outputs on the page, and
the suite calls it (`the_site_tour_prints_what_it_promises`, skipped
without python3).
Verified: corpora 4x162 = 648/648 byte-identical, suite 29/29, bootstrap
fixed point, fuzz 3000 clean, 0 leaks on the new native test. Version
0.6.0 in `Cargo.toml`, the README header and the site badge.

**Community + legal files, and THE CYCLES DECISION.** CODE_OF_CONDUCT.md
(Rust's, reformulated: acceptance-of-all promoted to the first rule and
extended to *level of experience* — "not knowing a word is an ordinary
state, asking is the right move"; concrete review conduct; report to
contact@geneacta.com). SECURITY.md with a real threat model: compiling
Keal source RUNS its author's code (native blocks + extern + cc
invocation), so the boundary is the operator's sandbox; in scope =
safe-program-to-unsafe-binary, checker unsoundness the native backend
trusts, compiler panics on any input, path escapes. Cargo.toml gained
license/repository; README gained "Taking part" + License sections.
TONY MUST FLIP ONE SETTING: Settings -> Security -> Private vulnerability
reporting -> Enable (the gh token lacks admin scope).
CYCLES — DECIDED (memory.md §5 rewritten with the full argument):
**weak references, NOT a collector.** Tony proposed the collector; the
challenge that won: (1) `deinit` is observable output, so a collector
the interpreters cannot run breaks three-engine agreement — and they
cannot run one, their values are Rust `Rc` whose decrements we do not
own (same rewrite threads.md declined); (2) trial deletion taxes every
decrement-that-misses-zero, i.e. cycle-free programs pay, and it cannot
be gated the way KEAL_ACTORS/try/deinit modes are; (3) deinit order
inside a cycle is arbitrary by construction, carving an exception into
the determinism deinit sells; (4) actors now run on real threads with
atomic counts — concurrent trial deletion is research-grade, stop-the-
world is the pause counting exists to avoid. weak wins because it is
implementable IDENTICALLY on all three engines: Rc::downgrade/upgrade in
interp+VM, strong/weak header natively (husk survives until the last
weak read; second count word paid only by programs that write `weak`).
The gap weak leaves (accidental cycles) is answered by DIAGNOSIS not
collection: a checker note where a `var` field's type can reach its own
class (suggesting `weak`), and an opt-in exit audit naming what survived
by type. A collector stays possible later — the weak header is most of
what it needs. Demonstrated first: a cycle's deinit never runs on any
engine (tests in /tmp only; the demo belongs in the weak commit).
DONE in `56a20ce`: `weak` is a contextual keyword on a class field whose
type is `T?` over a class; reading upgrades (`Rc::downgrade`/`upgrade` in
interp and VM, a strong/weak header behind `KEAL_WEAK` natively, emitted
only for programs that write the word), writing never retains, a class
holding one cannot be copied and so cannot cross into an actor. Two
things fell out: statement temps are on the same short leash under
`weak` as under `deinit` (a weak field makes *when* an object dies
observable), and rendering a cyclic value is a depth-capped panic on
every engine instead of a stack overflow. The checker cautions about one
shape: a class with a `deinit` and a mutable field able to point back at
its own object. Still open, as designed: the opt-in exit audit naming
what survived by type.

**`Any` natively — the last construct the C backend refused. DONE.**
Representation is memory.md §4 made real: `KealAny { const KealTypeInfo* ti;
KealWord w; }` — 16 bytes, tag + payload. KealTypeInfo = { name, retain,
release, show, eq }; statics keal_ti_int/float/bool/str/list in runtime.c,
per-class `K_X_ti` generated once by ensure_class_ti (identity eq, like the
interpreters compare dynamic instances). NULL ti == null Any (that's why
Any? collapses to Any). Inside containers a slot is one word and an Any is
two, so it boxes: KealAnyBox { rc; KealAny } with keal_any_box /
keal_any_box_release / keal_any_box_eq — Elem::Any (twin CElem kind "any")
carries this in elemWord/elemUnword/releaserThunk/elemEqFn.
ENTRY RULE (any_of/anyOf): only what one tag can name crosses — scalars,
String, List<Any>, class at arg-free-or-all-Any instantiation, and the
value-optionals via keal_any_of_opt_i64/f64/bool. A List<Int> is REFUSED
by name at the boundary ("only a value whose layout its tag can name
crosses") — that is the honest edge, pinned by native-unsupported/covered.
EXIT RULE (any_payload): `is` narrowing casts the payload BORROWED; the
tagged variable keeps the reference for the narrowed scope. Ident reads
consult declared==Any (locals) or the new `any_globals` set, filled by a
prepass over top-level Lets in program() because a function body compiles
before main's statements.
Wiring: coerced_to gained the Any arm (that's the single funnel — Let,
Return, args, fill_slot, list/map literals, add(), assign, map value,
elvis fallback all route through it); binary == on Any -> keal_any_eq
(null literal -> tag test); equality() Any arm for when-value arms;
elvis/notnull Any arms; to_string_value -> keal_any_display (bare string
stays bare), repr_call -> keal_any_repr (quoted, like the interpreters);
typeOf is dynamic on Any, a compile-time constant string elsewhere;
`?.` on an Any refused by name (pointer-shaped machinery).
when-arms: plain `is` = tag compare in arm_test; `is C(a,b)` needs its
own emitter (is_arm_with_binds / isArmWithBinds) because binds must be
visible to the guard AND the body — cast once, bind borrowed, guard
inside the tag test, `break` at the end.
JOIN SUBTLETY (cost me the first green): Any joins now take a slot, but
NOT in statement position — an if/else whose branches are statements
joins to Any and its "value" is never legitimately read (the checker
refuses using it). New `discard_join` flag set around a statement's
expression and cleared by the three join emitters; Any slots also
initialize to keal_any_null() and fill_slot runs valueless branches for
effect only. Without this, prelude flatMap failed to compile.
Checker: `is` on a FUNCTION TYPE is now refused ("a function's signature
has no run-time identity") — it was a real oracle/twin divergence the
map found: VM said true for any callable, interp always false.
FOUND+FIXED (serious, pre-existing since the scheduler commit):
program_uses_actors scanned the WHOLE program for ActorSystem/ActorRef/
copyClosure — and THE PRELUDE DECLARES THEM, so KEAL_ACTORS was defined
for every program and every native binary ran atomic refcounts. Declaring
is not using: the file that declares ActorSystem is excluded from the
scan now (oracle + twin). Audit is `grep KEAL_ACTORS` on emitted C:
core.keal 0, actors.keal 1. Measured cost of the bug: ~15% on a
refcount-saturated microbench, nothing measurable elsewhere — wrong
regardless; threads.md records it.
Tests: tests/native/any.keal + .expected (three engines: when/is arms,
is C(r) with guards, List<Any> incl. nested, ==, for+narrow, global
narrow inside a function, interpolation, ?: on Any, map values, typeOf,
ternary join), covered.keal rewritten to the new boundary. Verified:
corpora byte-identical four ways, 28/28, bootstrap fixed point, fuzz
3000 clean, 0 leaks. Docs: memory.md §4 rewritten (with its two costs),
types.md narrowing rules, language.md §20 (was badly stale) + `is`,
README.

**Threads stage 5 COMPLETE — the pthread scheduler. Actors run on real
OS threads under `keal build`.** The shape that made it small: the actor
classes stay ordinary compiled Keal; exactly FOUR method bodies are
generated instead of compiled (cbackend `actor_method_body`, twin
`actorMethodBody`, hook in method_named after the decl line, gated on
actors_mode): ActorRef.send / Outbox.post (deep-copy OUTSIDE the lock,
push + broadcast under it), Outbox.drain (snapshot under the lock, as
copies — prelude drain now copies too, so the engines agree), and
ActorSystem.run (per-monomorph `_actctx` typedef + `_actor_main` thread
fn into helpers; one pthread per actor; quiescence = every mailbox empty
AND workers==0, checked under the one global mutex `keal_actor_mu` with
one condvar, every push/completion broadcasts; join; then rethrow). The
runtime side (src/runtime.c, embedded for everyone): `#define
KEAL_ACTORS` — emitted by finish() only when actors_mode — flips
`keal_rc_t` to `_Atomic int64_t` and retain/release to
KEAL_RC_BUMP/KEAL_RC_DROP (fetch_sub — the dec-then-test pattern
double-frees under threads); without the define the macros are plain
++/-- and no pthread.h is included, so non-actor programs pay nothing.
WHY atomic at all: addresses (ActorRef/Outbox), strings shared by copy
(immutable), and immutable globals ARE visible from two threads;
memory.md updated honestly. Panics: a handler panic is captured per
thread (run wraps handler calls in keal_try_depth++ ONLY in catch_mode;
without any `try` the actor thread exits at the panic site with the
same stderr the deterministic run printed), carried in KealRunState
(first wins) with the NEW `keal_unwind_line` TLS, rethrown on the
thread that called run → `try { sys.run() }` catches on every engine.
ORDER MATTERS in _actor_main: capture panic + clear unwinding BEFORE
keal_drain_drops, or a queued deinit trips on its own guards (found by
tests/native/actor-panics.keal). deinit queue is thread-local → runs on
the actor's thread. -pthread added to nativebuild.rs + suite compiles.
Structs now say `keal_rc_t rc;` (oracle emit_struct + twin + runtime
structs). Tests: actor-panics (try/deinit/second run), actor-mesh
(8 actors full-mesh fan-out, order-insensitive total), suite test
`actors_are_clean_under_thread_sanitizer` (5 sanitized runs, skips
without cc/TSan). Verified: corpora 4-way byte-identical, 27/27,
bootstrap fixed point, fuzz 3000 clean, leaks 0, TSan 0, 100-run
stability. Docs: threads.md (decided→done), memory.md counts section,
prelude actor comments, README, TUTORIAL. Embeds regenerated (both).
JNI attach — DONE in the follow-up commit: keal_jvm_env and the
jvalue arg buffer are _Thread_local in lib/jvm.keal; keal_jvm_need does
GetEnv → AttachCurrentThread lazily and arms a pthread_key destructor
that detaches at thread exit (the CreateJavaVM thread is never
detached); jvmStart on a running VM now just attaches the caller.
examples/interop/java/actordate.keal + suite test
jvm_calls_work_from_actor_threads (JDK-gated). 28/28. No twin work —
the native block is verbatim data to both compilers. Stage 6
(measure) DONE: M4/10-core numbers in threads.md — ~6x compute scaling
on 8 actors, ~50ns/message single-actor, broadcast storm visible only
as sys time on the empty-handler flood; verdict recorded: nothing to
optimize yet, first lever if ever = per-mailbox condvar.

**Threads stage 3a COMPLETE — the actor ownership semantics:** spawn
copies captures per actor (copyClosure builtin: interp rebuilds env from
cbackend::collect_free + find_below_root/root_of on Scope; VM re-cells;
native per-lambda _copy fn behind the new KealClosure.copy slot, gated
on programUsesActors). send copies messages (prelude). Outbox<T> +
outbox() in prelude = main's result mailbox; ActorRef/Outbox blessed as
shared addresses in checker copyable, deep_copy, cbackend copyExprOf
(INCLUDING the Nullable(address) arm — that was a real bug: duplicated
mailboxes swallowed replies). Checker spawn rules: literal handler,
copyable captures, no `this`, no global var / mutable global val
(deeply_immutable); ActorSystem/Outbox ctor requires copyable M;
copy's Param refusal relaxed (instantiations settle it — native refuses
by name, interp at runtime); copy/copyClosure top-level shadowing
forbidden (only those two names). FOUND+FIXED pre-existing leak: empty
container literals in generic class bodies typed List<Never> (rigid
params mistaken for inference vars → params_rigid) → NULL release
thunks → every generic field list leaked elements natively. Tests:
actors rewritten to reply/Outbox patterns (programs+native+tutorial),
actor-rules.keal error corpus. Corpora 636/636, suite 26/26, bootstrap,
fuzz clean. NEXT: stage 4, the pthread scheduler — semantics frozen.

**copy natively (threads stage 2 complete) + repo polish:** the C
backend now GENERATES per-type copy fns (kcopy_<mangled>: lists/maps/
classes/nullables; memoized-before-body so recursive types close; depth
cap with the interpreters' exact message; unwind path releases the
partial copy — tests/native/copy.keal three-engine with cycle-catch and
deinit balance, 0 leaks). Twin mirrored byte-identically. threads.md
now records THE stage-3 decision: spawn will COPY handler captures per
actor (shared-capture aggregation is a data race under threads; all
three engines adopt the copying semantics together; actor tests get
rewritten to reply-patterns in that change). Also: 83 corpus tests
renamed to say what they prove; CONTRIBUTING.md; assets/ logo + .keal
file icon SVG.

**copy (threads stage 2, user-facing):** copy(value) — deep copy on
interp+VM (shared native.rs deep_copy, 10000-depth cap with an
identical catchable panic for cycles; a copy is a NEW object → deinit
runs for both). Checker special arm in check_call (before the builtin
table; copyable predicate refuses Fun/Any/Param/Self and walks class
fields with a visited list for recursive types); builtins table carries
the (Any)->Any signature for `val f = copy` (runtime re-checks). Native
REFUSES copy by name until stage 3 (per-type copy fns arrive with the
scheduler; send starts copying then, all engines at once). Twins
mirrored (checkCall arm + copyBlocker + builtins). Tests:
tests/programs/copy.keal, copy-refusals.

**Hardening round (user-driven):** differential fuzzer
tests/fuzz/fuzz.py (grammar generator, typed-by-construction + injected
mistakes + clean mode; asserts checker never crashes AND accepted
programs run byte-identically on both interpreters). 27k programs: zero
checker crashes; the diff mode found ONE real bug on its first run —
interp said "integer overflow in `+`" vs VM/native "integer overflow";
fixed in interp, pinned by tests/runtime/overflow.*. docs/types.md now
formalizes assignability/join/inference/narrowing incl. the honest
fill-and-join debt note. keal doc (src/kealdoc.rs, Rust-only tool like
bindgen): /// comments + parsed signatures → self-contained HTML; no
args = stdlib reference; snapshot test tests/doc/.

**Actors-on-threads: designed, staged (docs/threads.md).** Key call:
cross-actor ordering is unspecified, so the deterministic round-robin is
a LEGAL schedule — interpreters keep it forever; threads are native-only
behind the same run(). Messages deep-copy (generated copy_M, monomorph);
M must be closure/cell-free (checker predicate, refuse by name). Stage 1
DONE: runtime.c unwind + deinit state is _Thread_local (embed regen).
Stage 2 next: copy_M + message-safety check. Stage 3: the pthread
scheduler (locks/join/panic paths, TSan). JNI: AttachCurrentThread lazily.

**deinit (latest):** `proc deinit()` runs at refcount zero — queued,
drained at statement boundaries (Op::DrainDrops / keal_drain_drops();
runtime.rs shares one queue via Runtime::call_method and Instance's Drop
impl marks-before-queue to survive TLS teardown). Reverse-declaration
death order everywhere: interp Scope keeps insertion order and tears
down in reverse (HashMap order is random per process — this was a real
bug), VM pops frame stacks one-by-one and clears block slots (+ break/
continue paths) in reverse when the program has a deinit, native was
already reversed. Native also releases each statement's expression
TEMPS at its boundary in drop mode (they otherwise pin values to block
end — the r = Res(2) reassignment bug). Once-per-object via dropped/
kdropped set at queue time; resurrection survives; manual calls are
checker errors; jbind wrappers auto-free (released guard + deinit).
Everything gated: no deinit → byte-identical output. Named deinit, NOT
drop (drop is the take/drop pair). Tests: tests/programs/deinit.keal,
tests/native/deinit.*, deinit-rules. docs/drop.md has the exact semantics.

**Ternary + spaceship:** `c ? a : b` selects on Bool, three
branches select on a `Comp` (lazy, condition once — VM sign-splits via a
temp slot, native mirrors if_expr's slot mechanics in `ternary()`), and
`a <=> b` rewrites in the checker to the prelude's generic `compare(a,b)`
(NOTE the lastInst dance: after the in-place rewrite's inner checkExpr,
re-publish `lastInst = e.inst` or the outer wrapper erases the
instantiation and native emission loses the monomorphized name). Nested
ternary in the THEN position needs parentheses (a third colon reads as
the inner ternary's greater-branch); the else-position chain reads plain.
Tests: tests/programs/ternary.keal, tests/native/ternary.*, ternary-arity, ternary-missing-colon,
tutorial mirrors. Corpora 600/600, suite 25/25, bootstrap green.

**Exceptions, the shape that landed** (user asked for "gestion des
exceptions"): `throw "msg"` raises the same String panic every built-in
failure raises; `try { } catch (e) { }` (statement; `catch` may sit on
the next line, skip-semis like `else`) binds the message and continues.
`return`/`break`/`continue` pass through uncaught (VM pops a frame's
handlers on Return; the compiler emits PopHandler for breaks that jump
out of a try; `try{return}catch{return}` types as Never like if/else).
Interp: intercept `Flow::Err` in exec_stmt. VM: `Op::PushHandler/
PopHandler/Throw`, a `handlers` stack, and `execute` wraps
`execute_inner` in a catch loop (three truncations + jump; RC stays
exact because popped Values drop). **Native `try` — DONE too**
(checked unwinding / poisoned returns; docs/drop.md records the design
and the rejected ones). Zero cost when the program has no `try`
(`program_has_try` gates all of it; non-try corpus byte-identical).
Per-scope unwind labels release the ever-owned list (hoisted NULL
declarations), functions poison-return, `try` bodies chain to the catch
label; runtime keeps keal_try_depth/keal_unwinding, panicking helpers
return poison; JNI gateway bails after each check so Java exceptions
land in native `catch` (suite: java_exceptions_are_catchable_natively).
Twins fully mirrored — corpora 4x147 = 588/588. Tests:
tests/programs/exceptions.keal (+tutorial.keal), tests/native/trycatch
(3 engines, 0 leaks incl. unwind paths), trynative.keal (cgen corpus),
try-without-catch, throw-not-a-string. Remaining half: the `proc drop()` hook (interp/VM need a
deterministic pending-drop queue; see docs/drop.md and NEXT).

## Recently landed (kept for context)

**Operators `++ -- ** **= ^/ ^/=` + `Comp` type** (user-requested; `//`
for root was impossible — it is the line comment — so root is spelled `^/`).

Already done on the Rust side (compiles, VM+interp agree, `/tmp/ops.keal`
smoke test passes):
- Lexer tokens (`PlusPlus MinusMinus StarStar StarStarEq RootOp RootEq`),
  ASI-ends-statement for `++`/`--`.
- Parser: `**`/`^/` at binding power 8 **right-associative**; `**=`/`^/=`
  compound; `x++`/`x--` statements desugared to `x += 1`/`x -= 1`
  (op-token span carries the synthesized `Int(1)`).
- AST `BinOp::Pow/Root` (+`symbol()`), checker: `operator_trait` →
  (`Pow`,`pow`)/(`Root`,`root`); numeric lhs **rewrites `**`/`^/` to
  `.pow`/`.root` method calls** (compounds go through `binary_result`
  which gained Pow/Root in the arithmetic arm); `builtin_implements` Int and
  Float include Pow/Root.
- `runtime.rs`: shared `int_pow`/`int_root`/`float_root` used by interp
  `binary()`, VM `arith()` (new `Arith::Pow/Root`, compiler mapping,
  `arith_symbol`), and `native.rs` methods `Int.root`, `Float.root`
  (+ `builtins.rs` sigs). `runtime.c`: `keal_int_root`, `keal_f_root`.
- cbackend (oracle): compound Pow/Root arms (Int → `keal_int_pow/root`,
  Float → `pow`/`keal_f_root`), `Int.root`/`Float.root` method emission,
  and a new `builtin_operator_method` emitting plus/minus/times/div/rem/
  negate/equals/compareTo/pow/root on Int/Float/Str/Bool receivers (needed
  because bound generics like `compare<T: Ord>` call `compareTo` on `Int`).
- Prelude: traits `Pow`/`Root`; `record Comp(val sign: Int)` with
  isLess/isEqual/isGreater/isAtMost/isAtLeast/toString; generic
  `fun compare<T: Ord>(a, b): Comp`. Embeds regenerated.

**All remaining steps below were completed** (float printing fixed with a
shortest-roundtrip loop in `keal_str_from_float`; all four twins mirrored;
tests `tests/programs/operators2.keal`, `tests/programs/guarded.keal`,
`tests/native/powroot.keal`, `pow-root-misuse`/`increment-a-literal`; corpora 536/536; suite 21/21;
bootstrap fixed point green; README updated; version 0.5.0):
1. **Float printing in native**: `keal_str_from_float` uses `%g` (6
   digits) but Rust prints shortest-roundtrip → `2.0 ** 0.5` prints
   `1.41421` natively vs `1.4142135623730951`. Fix `runtime.c`: precision
   loop `%.{1..17}g` until `strtod` roundtrips; regen `runtimesrc.keal`.
   Then `/tmp/ops.keal` native run must equal the VM, leaks clean.
2. **Twins**: mirror everything —
   - `selfhost/lexing.keal`: the six new tokens (`++` `--` `**` `**=`
     `^/` `^/=`), max-munch order (`**=` before `**` before `*=`; `^/=`
     before `^/` before `^`), and `++`/`--` added to `endsStatement`.
   - `selfhost/parsing.keal`: `binaryOpOf`/`binaryPowerOf` gain `**`/`^/`
     at 8; right-assoc (parse rhs at `bp` not `bp+1` for those two);
     `isAssignOp`/`assignOpName` gain `**=`→`**`, `^/=`→`^/`; the
     `x++`/`x--` statement desugar identical to Rust (same spans!).
   - `selfhost/checking.keal`: `operatorTrait` + Pow/Root;
     `builtinImplements`; the numeric-lhs rewrite in `checkBinary`;
     `rewriteOperator` arm; `binaryResult` arithmetic arm.
   - `selfhost/cbackend.keal`: compound arms, root method emissions,
     and the `builtinOperatorMethod` mirror (exact same C strings).
3. **Tests**: `tests/programs/operators2.keal` (asserts: ++/--/on fields
   and elements, ** right-assoc, ^/ int+float, compounds, Comp/compare,
   literal widening `x ** 2` on Float); `tests/native/powroot.keal`
   (three-engine prints incl. `2.0 ** 0.5`); error cases in
   `tests/selfhost/type-errors/pow-root-misuse.keal` (`"s" ** 2`, `true ^/ 2`,
   class without Pow, negative Int root/pow panics are runtime not check).
4. **Corpora ×4 + suite + bootstrap** green; README operator table update
   (the answer text from the conversation is the source), TUTORIAL touch.
5. Commit + push (author rule!), update this file.

**Guarded return — DONE** exactly as designed (both parsers, lookahead on
the `{` after the condition keeps the if-expression form; guard narrowing
works through it). **Version 0.5.0 — DONE** (Cargo.toml + README header).

## NEXT after the in-flight work (user-agreed roadmap)

1. **`keal jbind` — DONE** (src/jbind.rs; `keal jbind [--jvm <path>]
   <java.Class|saved.javap>...`; gateway gained `jvmNew`/`jvmCallInt`/
   `jvmStaticInt` because JNI requires exact-return-type calls; wrappers
   hold `val handle: Int` + `free()`, a representable `compareTo(Self)`
   makes the class `Ord`; snapshot test `tests/jbind/` is JDK-free, the
   E2E test generates live and builds native). Next: **loader sugar** — a
   non-path `import java.time.LocalDate` invokes jbind and caches the
   module — the endpoint the user confirmed. Note: wrappers need manual
   `free()`; an on-release drop hook is a language feature to design
   (interp/VM would have to call user code at refcount zero — decide
   carefully). Go demo (c-archive) worth adding.
   **Loader sugar — DONE too**: `import java.time.LocalDate,
   java.time.DayOfWeek` desugars IN BOTH PARSERS to
   `.jbind/<classes joined with +>.keal`; classes in one import are bound
   together. `keal run`/`check`/`build`/`layout`/`emit-header` use
   `loader::load_generating` (fills the cache via `javap`; the dir gets
   its own `jvm.keal` copy so a committed cache builds without a JDK);
   the four dump commands and the twins NEVER generate — cache-only —
   so the corpora stay pure (missing cache → identical error + hint in
   oracle and twin loader). Corpus: `tests/selfhost/jbindsugar.keal` +
   committed `tests/selfhost/.jbind/`, `import-trailing-dot`, `jbind-cache-missing`; suite test
   `import_sugar_works_end_to_end` (JDK-gated).
2. Closure callbacks at the C boundary (interop 1d) once a consumer fixes
   the `userdata` convention.
3. Threaded actors — DONE through stage 6 (scheduler, JNI attach,
   measurements recorded in threads.md). `Any` natively — DONE. `weak` —
   DONE. Visibility, namespaces and dependencies — DONE. Typed exceptions
   — DONE, all three engines. What is left, in order: a rule telling an
   accidental cycle from a global that lived to the end (the audit reports
   both alike today); a registry, if it is ever worth one; `constexpr`
   evaluation; macros last. See README "What remains".

## Key file map

`src/` Rust oracle (lexer/parser/checker/cbackend/interp/vm/compiler/
runtime.rs shared semantics/runtime.c native runtime/prelude.keal).
`selfhost/` the Keal compiler (lexing/parsing/ast/astprint/checking/
checker=driver/cbackend/loader/types/builtins + generated preludesrc,
runtimesrc). `lib/jvm.keal` JVM gateway. `tests/` corpora (native,
native-extern, selfhost/{errors,parse-errors,type-errors}, programs,
bindgen). `bootstrap.sh` → `dist/kealc`. Suite: `tests/suite.rs`.
