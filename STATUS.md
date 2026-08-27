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

Nothing — operators/Comp/guarded-return AND `keal jbind` are **DONE and
pushed**. Next up is the loader sugar `import java.time.LocalDate`
(see NEXT).

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
`tests/native/powroot.keal`, `te36`/`perr29`; corpora 536/536; suite 21/21;
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
   `tests/selfhost/type-errors/te36.keal` (`"s" ** 2`, `true ^/ 2`,
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
