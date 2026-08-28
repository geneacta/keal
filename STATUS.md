# STATUS — where the work stands, and how to resume it

*Updated: 2026-08-27. This file is the hand-off: if a session dies, the next
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
   raw-string `fun ...Source(): String`), **after** `cargo build --release`
   (the binary embeds them via `include_str!`).
4. **Match or refuse**: the native backend never mis-compiles — it refuses
   by name. Runtime behavior must match the interpreters (messages
   included); `tests/native/*` runs on all three engines.
5. **Verification loop** (run before every commit):
   - the four-corpora loop (see below), `cargo test --release` (currently
     21 green incl. bootstrap fixed point), `./bootstrap.sh`.
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

Nothing — operators/Comp/guarded-return, `keal jbind`, the loader sugar
`import java.time.LocalDate`, the verified Go demo, exceptions
(`throw` / `try`-`catch` on all three engines incl. native checked
unwinding), the six-language polyglot demo AND the ternary family are
**DONE and pushed**. See NEXT.

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
3. Threaded actors (same API, one heap per actor); `Any` natively (RTTI);
   cycles decision; `constexpr`; macros last. See README "What remains".

## Key file map

`src/` Rust oracle (lexer/parser/checker/cbackend/interp/vm/compiler/
runtime.rs shared semantics/runtime.c native runtime/prelude.keal).
`selfhost/` the Keal compiler (lexing/parsing/ast/astprint/checking/
checker=driver/cbackend/loader/types/builtins + generated preludesrc,
runtimesrc). `lib/jvm.keal` JVM gateway. `tests/` corpora (native,
native-extern, selfhost/{errors,parse-errors,type-errors}, programs,
bindgen). `bootstrap.sh` → `dist/kealc`. Suite: `tests/suite.rs`.
