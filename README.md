<p align="center"><img src="assets/keal.png" alt="Keal" width="360"></p>

# Keal — a statically typed language for programs that have to be predictable

**Version 1.0.0** · `keal version` prints the toolchain's own.

Kotlin's shape over a C-family syntax, compiled to native code, with
**deterministic destruction and no garbage collector** — and no borrow
checker to argue with. An object dies when its last reference does, at a
statement boundary you can point at, and `deinit` runs there. No pause, no
generation, no lifetime annotation.

Three things hold it to that, and they are the reason to look twice:

* **Three engines, one answer.** A tree-walking interpreter (the
  specification), a bytecode VM (the default), and a native compiler
  through C11. The suite runs every program through all three and compares
  what they print, byte for byte.
* **Match or refuse.** The native backend never mis-compiles. What it
  cannot compile, it names — and the tests check that the list of names is
  the truth.
* **Written twice, and held to itself.** The compiler exists as a Rust
  reference and as a compiler written in Keal, and the two must agree
  byte-for-byte on the tokens, the tree, the types and the C of every file
  in the repository. Then it compiles itself to a fixed point.

Zero dependencies, and it compiles through C — so it builds wherever a C
compiler does. The whole language fits in one sitting of
[TUTORIAL.md](TUTORIAL.md); [CONTRIBUTING.md](CONTRIBUTING.md) is the
procedure, and the three commands that verify it.

At a glance:

* **Type inference, null safety, smart casts** — Kotlin's shape over a
  C-family syntax
* **Pattern matching** (`when` / `is` / guards / destructuring), tuples,
  records
* **Generics by monomorphisation** (no erasure, no boxing), traits with
  default methods, operator overloading
* **Lazy sequences** (Stream/Sequence-style, written in Keal itself) and an
  **actor model** for concurrency — deterministic on the interpreters, a
  pool of OS threads under `keal build`, same output either way. Two
  thousand actors cost as many threads as the machine has cores, not two
  thousand
* **Modules that keep their own counsel** — a declaration is private to its
  file unless it says `package` (the files beside it) or `public`, and two
  modules may declare the same name: `import "./config.keal" as config`.
  Dependencies are git repositories at an exact commit, named in
  `keal.toml` and fetched with `keal fetch`. `keal search` and `keal add`
  find one and pin it, reading an index that is itself a git repository —
  no service, and nothing that breaks if it disappears
* **Nothing changes behind your back** — a parameter cannot be reassigned,
  and what it *holds* belongs to the caller unless the signature says `var`.
  The checker refuses every way around it, a call to something that would
  included
* **A language server** — `keal lsp`, one binary for VS Code, Neovim, Helix
  and Zed: diagnostics as you type, hover types, go to definition, find
  references, rename, outline, completion. It reuses the compiler rather
  than modelling the language again
* **Enums that close a `when`** — `enum Suit { Hearts, Spades }`, and a
  `when` over one needs no `else`. Add a variant and every `when` that
  forgot it is an error, in statement position too
* **Macros that are syntax, not text** — `swap!(a, b)` rebinds both names,
  an argument may run twice or never, and a `return` inside one returns from
  the function around it. Hygiene by scoping: what a macro binds lives in a
  block of its own
* **Reference counting** with a fully documented memory model — inspect any
  program's layout with `keal layout`
* **Native compilation** (`keal build`) ~84× faster than the VM, with **C and
  C++ interop** built in — and a
  [staged plan](docs/interop.md) for Rust, Go, Java and Kotlin
* **Self-hosted lexer, parser, type checker and code generator**, each held
  to byte-for-byte agreement with its Rust oracle

```keal
class Point(val x: Float, val y: Float) {
    func length(): Float { sqrt(this.x * this.x + this.y * this.y) }
    func toString(): String { "(${this.x}, ${this.y})" }
}

func firstLong(points: List<Point>, min: Float): Point? {
    for (p in points) {
        if (p.length() > min) { return p }
    }
    return null
}

val points = [Point(1.0, 1.0), Point(3.0, 4.0)]
val found = firstLong(points, 2.0)

println(when {
    found == null -> "nothing long enough"
    else -> "${found} has length ${found.length()}"
})
```

```
$ keal hello.keal
(3.0, 4.0) has length 5.0
```

## Getting started

```sh
git clone https://github.com/geneacta/keal.git
cd keal
cargo build --release
```

That leaves a binary at `./target/release/keal`. To put it on your path:

```sh
cargo install --path .
```

Write a file and run it:

```sh
$ echo 'println("hello, world")' > hello.keal
$ keal hello.keal
hello, world
```

A script can name its own interpreter, so it can be executed directly:

```keal
#!/usr/bin/env keal
println("hello from a script")
```

```sh
chmod +x hello.keal
./hello.keal
```

The rest of the commands:

```sh
keal check src/main.keal        # type-check without running
keal layout src/main.keal       # show how the program's values are laid out
keal build src/main.keal        # compile to a native executable
keal emit-c src/main.keal       # print the C that build would compile
keal repl                       # interactive session
keal --ast program.keal         # run on the tree-walker instead of the VM
keal tokens|ast|types|cgen f    # dump each compiler stage (the self-hosting oracles)
keal version
```

And the flagship: **build the compiler with itself.**

```sh
$ ./bootstrap.sh
1/4 building the Rust toolchain (the oracle)...
2/4 compiling the self-hosted compiler to native...
3/4 verifying the fixed point...
4/4 installing...
dist/kealc — the Keal compiler, written in Keal, compiled by itself.

$ dist/kealc program.keal > program.c && cc -O2 -std=c11 -o program program.c
```

`kealc` compiles the whole nine-module Keal compiler in about 0.2 seconds.

**Editing Keal?** [`editors/`](editors/README.md) has a VS Code extension —
highlighting for the whole language, snippets, and the compiler's own errors
reported inline. Installing it is one symlink, and it then follows the
repository:

```sh
ln -s "$PWD/editors/vscode" ~/.vscode/extensions/keal
```

Reload VS Code and open any `.keal` file. The grammar is a plain TextMate
file, which JetBrains IDEs and Sublime Text read as well;
[`editors/README.md`](editors/README.md) has the steps for each, the `.vsix`
package, and the check task that puts diagnostics in the Problems panel.

**New to the language? Start with the [tutorial](TUTORIAL.md)** — a guided
walk through everything, in about half an hour. Every snippet in it is
checked by the test suite.

`cargo test` runs the whole suite: self-checking programs, every example, the
tutorial, and snapshot tests for the diagnostics — each on **both** execution
engines, which must agree on every byte they print.

## What the language has

- **Static types with inference.** `Int`, `Float`, `Bool`, `String`, `Unit`,
  `List<T>`, `Map<K, V>`, `Range`, function types, `Any`, `Nothing`. You
  annotate parameters and fields; everything else is inferred. `Int`, `Float`
  and `Bool` are values and copy on assignment; `String`, `List`, `Map` and
  instances are shared references. There is nothing to box or unbox, and the
  base library declares no classes at all — only eight operator traits.
- **Null safety.** `T?` is a distinct type. `?.`, `?:` and `!!` get you across,
  and the checker smart-casts an immutable binding after `if (x != null)`,
  after an early-return guard, and across `&&`.
- **Classes and records.** Primary constructors that declare fields, field
  initializers, methods, and a `toString` hook that `println` respects. A
  `record` is the data case: immutable fields and `==` comparing them one by
  one, which is what makes a separate `struct` unnecessary.
- **`func` and `proc`.** A `func` must declare what it returns; a `proc` returns
  nothing. The split means `Unit` and `void` are never written by hand, and
  using a `proc`'s result is an error rather than a silent no-op.
- **Operator overloading through traits.** `+`, `-`, `*`, `/`, `%`, `**`
  (power, right-associative), `^/` (root — the inverse of `**`; `//` belongs
  to comments), unary `-`, `==` and the four comparisons are wired to
  prelude traits (`Add`, `Pow`, `Root`, `Ord`, …). The built-in types
  implement them too, so `func total<T: Add>(...)` accepts `Int` and your own
  type alike. Compound forms (`+=` … `**=`, `^/=`) and the statement
  increments `x++` / `x--` come along. Integer arithmetic is **checked** —
  overflow, division by zero and `Int.pow`'s negative exponent panic instead
  of wrapping — while `Float` follows IEEE 754 (`inf`, `NaN`, no panics).
- **`Comp`, the three-valued comparison — with its own ternary.**
  `a <=> b` (the spaceship) works on any `Ord` type and answers a `Comp`:
  `less`, `equal` or `greater` — `Comp` is to ordering what `Bool` is to
  truth. And the ternary knows both: `c ? a : b` selects on a `Bool`,
  `a <=> b ? smaller : same : bigger` selects on a `Comp`, lazily, the
  condition evaluated exactly once.
- **Guarded returns.** `return if (a > b) a` returns only when the guard
  holds and falls through otherwise — and the guard narrows, so
  `return unless (s == null) s` hands back a plain `String`.
- **`weak` fields.** A field that points without holding on, so the back
  edge of a cycle can be written and still die: `weak var owner: Owner?`.
  Reading gives the target while it lives and `null` the moment it goes.
  Programs that never write it pay nothing — objects keep their single
  count.
- **`keal doctor`.** The interop toolchains found on this machine — C,
  Rust, Go, JDK, Kotlin — next to the versions the test suite was last
  verified against. Versions are pinned, toolchains are not vendored.
- **`keal doc`.** The `///` comments and the compiler's own signatures,
  rendered to one self-contained HTML page — run it with no arguments
  and it documents the standard library. KealDoc, if you like.
- **`deinit`, deterministic.** A class may declare `proc deinit()`; it
  runs when the object's last reference dies — at the next statement
  boundary, in reverse-declaration order, exactly once, identically on
  all three engines (the semantics are pinned in
  [`docs/drop.md`](docs/drop.md)). `keal jbind`'s wrappers use it to
  free their JVM handles by themselves. A program that declares none
  pays nothing.
- **A checker that suggests, not just refuses.** Warnings that never fail
  the build: write `!(a and b)` and it answers *"`not (a and b)` is
  `a nand b`"* — the negated connectives have first-class names, and the
  checker teaches them (and the way back: `!(a nand b)` suggests plain
  `and`).
- **Exceptions, the honest way.** `throw "message"` raises the same panic
  every built-in failure raises, and `try { ... } catch (e) { ... }` binds
  the message and continues — one form covers your throws, overflow,
  division by zero, a failed `assert`, even a Java exception crossing the
  JVM gateway. `return` passes through uncaught. All three engines catch,
  byte-for-byte alike: the C backend unwinds by checked, poisoned returns
  — every scope releases exactly what it owns on the way out, zero leaks,
  zero cost to programs that never `try`
  ([`docs/drop.md`](docs/drop.md) records the design).
- **Generics and traits.** `func first<T>(...)`, `class Box<T>`, inferred one
  argument at a time so a later lambda knows what an earlier argument fixed.
  Traits carry required and default methods, `Self`, and bounds (`<T: Show +
  Ordered>`) that the checker enforces.
- **Eight logical connectives**, spelled as words: `not`, `and`, `or`, `xor`,
  `xnor`, `nand`, `nor`, `implies` (`!`, `&&`, `||` and `^` are accepted
  aliases). The ones that can short-circuit do; `xor` and `xnor` cannot and
  say so. **None binds tighter than another** — see below.
- **Functions as values.** Lambdas with inferred parameter types and an
  implicit `it`, closures that capture variables, nested functions, default
  and named arguments.
- **Expression-oriented.** A block's value is its last expression, so `if`,
  `unless`, `when` and function bodies all produce values without ceremony.
  `unless (c)` is `if (not c)` — the same construct, including `else`
  branches and smart casts.
- **`when`**, which is the switch and the match in one: values, ranges, type
  tests that narrow the subject, guards (`is Circle(r) if (r > 10.0) ->`), and
  a subject-less form that replaces a chain of `if`. No fall-through, first
  match wins, and it is an expression.
- **Destructuring and tuples.** `val Point(x, y) = p` names a value's
  constructor fields, and `is Circle(r) ->` tests and binds in one move.
  `func divmod(a: Int, b: Int): (Int, Int) { return a / b, a % b }` returns
  several values of different types, taken apart with `val (q, r) = ...`.
- **Lazy sequences** — the `Stream`/`Sequence` pipeline, written in ordinary
  Keal in the prelude: `seq(xs).map(f).filter(p).take(3).toList()` computes
  only what the terminal pulls, and `iterate(1, { it * 2 })` is an infinite
  source. Pull-based, fusing, zero cost for programs that never use it, and
  it compiles to native like everything else.
- **Actors** — the concurrency model the language committed to: `spawn` a
  handler, `send` messages, `run`. The interpreters deliver round-robin,
  deterministically; compiled natively each actor is an OS thread, joined
  at quiescence and verified under ThreadSanitizer. One heap of truth per
  actor (state lives in the handler's own copy of its captures), messages
  deep-copied, the same output on every engine for every program that only
  depends on the order the model actually promises.
- **A standard library** of about ninety built-ins over strings, lists, maps
  and numbers, including `map`/`filter`/`fold` typed generically.
- **Modules.** `import "./other.keal"`, resolved relative to the importing
  file and loaded at most once.
- **Diagnostics** that quote the source, point at the column, and suggest a
  fix — sorted by position, with as many independent errors per run as the
  checker can find.

[`TUTORIAL.md`](TUTORIAL.md) is the guided tour, [`docs/language.md`](docs/language.md)
the complete reference, and [`examples/`](examples/) holds working programs —
including [an arithmetic evaluator written in Keal](examples/calculator.keal):
tokenizer, precedence parser and evaluation in about 120 lines.

## How it is put together

```
src/
  lexer.rs     tokens; automatic semicolon insertion; string interpolation
  parser.rs    recursive descent, precedence climbing for binary operators
  ast.rs       the syntax tree
  types.rs     the type lattice: assignability, joins, nullability,
               substitution and unification for generics
  builtins.rs  type signatures for the standard library
  checker.rs   name resolution, type checking, null-safety analysis
  value.rs     runtime values, environments
  layout.rs    the memory model: what a value is, in bytes
  cbackend.rs  the native backend: checked AST -> C
  runtime.c    the C runtime: reference counting, strings, checked arithmetic
  nativebuild.rs  driving a C compiler over the emitted C
  bytecode.rs  the instruction set
  compiler.rs  AST -> bytecode: name resolution and capture analysis
  vm.rs        the bytecode virtual machine
  interp.rs    the tree-walking evaluator, kept as the reference
  runtime.rs   what both engines share: errors, rendering, indexing
  native.rs    runtime implementations of the standard library
  prelude.keal the operator traits, written in Keal and compiled in
  loader.rs    module resolution
  repl.rs      interactive session
  span.rs      source locations and diagnostic rendering
```

The pipeline is source → tokens → AST → checked AST → bytecode → execution.

**Two engines, one of which is the specification.** The bytecode VM is what
runs by default. The tree-walking evaluator is still there, reachable with
`--ast`, and the test suite runs every program through both and compares what
they print — down to the call stack of a runtime error. The evaluator is
simple enough to read as a specification, so a disagreement is a bug in the VM
until shown otherwise.

`constexpr` did **not** turn out to be that evaluator with a flag, and the
reason is worth stating: what a `constexpr` may do has to be a promise a
reader can hold in their head, and it has to be written twice — once in
Rust, once in Keal — without the two drifting. So it is a small evaluator
over a small language, and everything outside that language is refused by
name. `src/constfold.rs` and `selfhost/constfold.keal` are the two copies,
and the corpus holds them to the same answers and the same diagnostics.

The VM is a stack machine, and the speedup comes from two analyses moved into
the compiler rather than from the dispatch loop:

* **Names are resolved at compile time.** A local becomes a frame slot, a
  global an index. The evaluator hashed a string and walked a scope chain for
  every variable it read.
* **Only captured variables are boxed.** Before compiling a body, the compiler
  collects the names any nested function mentions; those live in cells, and
  every other local is a plain slot. The set is deliberately an
  over-approximation — boxing a variable no closure reaches costs an
  allocation, not correctness.

On the programs in `bench/`, that is 2–3× faster than walking the tree:

| | tree-walker | bytecode VM |
|---|---|---|
| `fib(30)` | 0.58s | 0.23s |
| 10M-iteration loop | 1.27s | 0.46s |
| 1M records built and read | 0.82s | 0.25s |
| map/filter/fold over 1M | 0.37s | 0.18s |

Getting substantially past that means a register-based instruction set,
unchecked dispatch, or a smaller value representation — each a project of its
own rather than a tuning pass.

Two design notes worth knowing:

**Semicolon insertion.** A newline ends a statement when the preceding token
could end one, the way Go does it. The cost is that an opening brace must
share a line with its construct; the benefit is a newline-insensitive grammar
and no semicolons in normal code.

**Inference argument by argument.** `xs.fold(0, { acc, n -> acc + n })` needs
the accumulator's type before it can type the lambda. Both the built-in table
and user generics resolve a signature after each argument they check, so
whatever an earlier argument settles is available to a later one.

**No `void`.** A declaration is either a `func`, which must say what it
returns, or a `proc`, which returns nothing — nothing in between. So there is
no annotation meaning "no result", and no way to accidentally consume one.

**Logical operators have no relative precedence.** `a or b and c` is a syntax
error in Keal; the parentheses are required. Most languages rank `and` above
`or` by a convention inherited from arithmetic, but with eight connectives
that convention stops carrying its weight — nobody reliably knows how `nand`
ranks against `implies`. Rather than invent an order and expect it to be
remembered, Keal asks. Repeating one connective is still allowed where it
cannot change the meaning, so `a and b and c` is fine while
`a nand b nand c` is not.

**Monomorphisation decided up front.** Generics solve to concrete types at
every call site, and the checker refuses a call it cannot fully solve. That
rules out a generic function as a value and `is T` on a type parameter — both
would need a uniform boxed representation, which a monomorphising backend does
not have. Choosing this now avoids designing the type checker twice.

## Where it is going

The target is native code. That makes one question unavoidable which an
interpreter can dodge — what a value is in bytes, and who frees it — and it is
answered in [`docs/memory.md`](docs/memory.md). The short version:

* **Reference counting.** Every heap object carries its count. No collector,
  no ownership annotations, no borrow checker.
* **Fields keep declaration order**, padding and all, because a struct whose
  shape the author cannot predict is one that cannot be handed to C.
* **`T?` is free where a spare bit pattern exists.** `String?` is the size of
  `String`; `Int?` is twice the size of `Int`, and says so.

`keal layout file.keal` prints the whole table for any program, so none of it
has to be taken on trust:

```
record Point
  24 bytes, align 8, 0 of which is padding
  offset  size  field                as
       0     8  <reference count>    usize
       8     8  x: Float             f64
      16     8  y: Float             f64
```

The layouts of a sample program are snapshotted in the test suite, so changing
a representation shows up as a diff rather than as a surprise.

### Compiling to native code

`keal build file.keal` produces a real executable. The route is through C —
the backend emits one self-contained translation unit and hands it to `cc` —
which buys machine code and the C interop the language wants at once, since
the output *is* C. Cranelift or LLVM can replace that step later; the
decisions that are hard to change live in `layout.rs`, not in the emitter.

```
$ keal build fib.keal
fib
$ ./fib
9227465
```

On `fib(35)`, against the same program on the other two engines:

| | |
|---|---|
| tree-walking evaluator | 6.14s |
| bytecode VM | 2.51s |
| **native, via C** | **0.03s** |

That is 84× the VM and 205× the evaluator. The generated code carries the same
guarantees: integer overflow is checked rather than wrapped, so a program
fails where it would have failed on either interpreter.

**Calling C and C++ from Keal** is part of the language, not an FFI bolted
on. A `native` block passes text into the generated C verbatim — a header, or
an implementation written inline — and `extern func` binds a symbol with a
signature the checker holds callers to. Only ownership-free types cross
(`Int`, `Float`, `Bool`), which is the boundary `docs/memory.md` drew before
any of this existed. C++ lives in its own files behind `extern "C"`, and
`keal build program.keal impl.cpp` compiles and links the lot, switching the
linker to `c++` when it has to:

```keal
native """
#include <math.h>
static int64_t triple(int64_t n) { return n * 3; }
extern int64_t fib_cpp(int64_t n);
"""

extern func sin(x: Float): Float
extern func triple(n: Int): Int
extern func fib_cpp(n: Int): Int
```

The interpreters refuse an extern call by name — `compile with keal build to
call into C` — rather than pretending.

**The boundary is wider than scalars.** A `String` crosses once its
ownership is written down — `borrow String` into C (the callee reads, must
not keep), `own String` back from it (Keal adopts the malloc'd buffer and
frees it) — and a record of bare values crosses **by copy** as a headerless
mirror struct `Keal_Name` the generated C defines for the native blocks to
use. The other direction works too: every non-generic Keal function with a
clean signature is an external `k_name` symbol, and `keal emit-header`
prints the C header for all of it, so a companion `.c` file can call
straight back into the program:

```keal
record Vec2(val x: Float, val y: Float)
native """
extern int64_t k_bonus(int64_t n);                       // Keal, from C
static double dot(Keal_Vec2 a, Keal_Vec2 b) { return a.x*b.x + a.y*b.y; }
static char* shout(const char* s) { /* malloc'd upper-case copy */ }
"""
func bonus(n: Int): Int { return n + 58 }
extern func dot(a: Vec2, b: Vec2): Float
extern func shout(s: borrow String): own String
```

Misuse is a checked error with the fix in the note: a bare `String` at the
boundary says *"write `borrow String`: C reads the bytes and must not keep
them."*

**And Rust works today, in four commands.** `keal build` takes link inputs
(`.a`/`.so`/`.o`, `-l`, `-L`) and compile flags (`-I`, `-D`), and
`keal bindgen header.h` turns a C header into `extern func` declarations —
binding exactly what crosses and skipping the rest *with the reason
printed*. A Rust staticlib's `cbindgen` header, a Go `c-archive` header, or
sqlite's own: same tool.

```sh
cargo build --release                      # Rust staticlib, extern "C" exports
cbindgen --lang c --output demo.h
keal bindgen demo.h > bindings.keal
keal build main.keal target/release/libdemo.a -I.
```

The verified demo lives in [`examples/interop/rust/`](examples/interop/rust/);
the staged path onward — Go, Java, Kotlin — is
[`docs/interop.md`](docs/interop.md).

**The backend covers most of the language.** Functions, control flow, `Int`,
`Float`, `Bool`, `String`, classes and records with their methods, nullable
references, `when`, `List<T>` with the same bounds panics as the interpreters,
lambdas — a lambda compiles to a C function plus a counted environment,
captures are `val`s taken by value, and `map`/`filter`/`fold`/`forEach`
become plain loops feeding each element through the closure — and **generics,
by monomorphisation**: the checker records the solved type arguments on every
call, and the backend emits one C copy of the function or class per distinct
set, on demand. `firstOr<T>` used at `Int` and `String` becomes two plain C
functions; `Boxed<T>` and the tuples become one struct per instantiation,
each with its own retain, release, equality and rendering. `Any` compiles
too, as the tag-and-payload pair [`docs/memory.md`](docs/memory.md) §4
always priced it at: `is` is a tag compare, `typeOf` reads the tag's name,
and narrowing casts the payload. What a tag cannot name — a `List<Int>`,
whose elements have their own stride — is refused at the boundary rather
than mis-shaped. Everything else landed — `Map`, default and named arguments,
generic methods, `var` capture through shared heap cells, `Int?` as the
tagged struct `keal layout` always priced it at, and the host trio
(`args()`, `readFile`, `writeFile`, `exit`) that self-hosting stands on.
Anything it cannot compile is **named**, not mis-compiled:

```
error: the C backend cannot compile list literals yet
  --> shapes.keal:9:22
  = note: run it on the bytecode VM instead, which supports the whole language
```

A class becomes a struct headed by its reference count, with its fields in the
order `keal layout` reports. The last reference to an object releases each of
the references it held, then frees it. A `T?` over a reference is the same
pointer, allowed to be null, so `?.`, `?:` and `!!` cost a branch and nothing
else — exactly what the layout table promised. `leaks` reports nothing
outstanding on the test programs.

The test suite compiles a program with a real C compiler, runs it, and
requires its output to match both interpreters byte for byte. That test is
what found the first two bugs in this backend.

## Self-hosting: the compiler is written in Keal

The whole pipeline exists twice, and the two copies are held together by
force. [`selfhost/lexer.keal`](selfhost/lexer.keal) is the Keal lexer written
in Keal, [`selfhost/parser.keal`](selfhost/parser.keal) the parser building a
real tree, [`selfhost/checker.keal`](selfhost/checker.keal) the type checker —
scopes, inference, generics solved per call site, traits, smart casts,
operator rewrites, null safety, its own module loader and embedded prelude —
and [`selfhost/cbackend.keal`](selfhost/cbackend.keal) the C emitter:
monomorphisation, ownership scopes, closures, niche-optimised nullables, the
same named refusals. The suite holds all four to **byte-for-byte agreement**
with the Rust originals — every file in the repository, every way each stage
can fail, spans, messages, notes and exit codes included, themselves
included. `keal tokens`, `keal ast`, `keal types` and `keal cgen` print the
oracles either side can be compared against.

**And the loop is closed.** `./bootstrap.sh` compiles the self-hosted
compiler to native, runs it on its own source, and verifies it emits **the
very C it was built from** — then installs it as `dist/kealc`. The suite
re-proves the fixed point on every run (`the_compiler_compiles_itself`).
Being its own first serious user is also what hardened the backend: the
UTF-8 string surface, structural list and map equality, numeric parsing —
each held to the interpreters' semantics by three-engine tests, with zero
leaks under macOS `leaks`.

## What remains

The honest list, in rough order of intent. (Interop used to live here;
it doesn't anymore — C, C++, Rust, Go, Java and Kotlin all answer from one
Keal file in [`examples/interop/polyglot/`](examples/interop/polyglot/),
and [`docs/interop.md`](docs/interop.md) tells that whole story. Exceptions
used to live here too; all three engines catch them now. So did actors on
real threads: `keal build` runs them on a pool of workers, TSan-clean,
and [`docs/threads.md`](docs/threads.md) is the record of how. So did `Any`
natively — the tag-and-payload pair, `is` as a tag compare — and `weak`,
which is how the back edge of a cycle is written so that the whole cycle
dies on schedule and every `deinit` runs. And so did the whole module
question: a declaration is private to its file unless it says `package` or
`public`, class members included, and two modules may declare the same name
because an import can be given one. Dependencies followed, transitivity and
lockfile included. And the last place the three engines could be told apart
closed: they now report the same objects outliving the same program, which
took four separate fixes and a machine nobody here owns. And the audit
stopped being a list to interpret: it names which survivors are a cycle and
which a top-level binding is holding on purpose. Macros used to sit at the
bottom of this list, marked *deliberately last*, and they were: `swap!(a, b)`
rebinds both names, `twice!(n += 5)` runs its argument twice and
`discard!(x)` never runs it at all, and a `return` inside one returns from
the function it was written in — three things a call cannot do, which is
what the `!` is telling you. A signature can promise not
to change what it was given: the contents of a parameter belong to whoever
passed them unless it says `var`, and the checker refuses every way of
breaking that — including handing the value on to something that would.
Packages gained the last
thing they were missing — a way to find one whose URL you do not know —
without gaining a service anybody has to run: `keal search` and `keal add`
read an index that is an ordinary git repository, one small file per
package, saying where that package lives and nothing else. And `constexpr`
arrived:
a binding the compiler computes and writes back as a literal, or refuses by
name — with a step budget, because a compiler that never answers is not a
tool. Typed exceptions
were the last thing on this list to be half-done, and are not anymore:
`keal build` carries the thrown value through the C unwind, so
`catch (e: Refused)` means the same thing on all three engines.)

* **Cycles across several classes still leak silently** — `weak` breaks
  the ones you can see, and the checker cautions about the shape that
  voids a `deinit`; a cycle nobody marked is still never freed. What
  exists is a verdict, on all three engines: `KEAL_AUDIT=1 keal run
  prog.keal` on the interpreters, `keal build --audit` for a compiled one,
  and the same report either way — what outlived the program, by type, and
  which of it is a cycle. A mark phase with no sweep runs at exit: what a
  top-level binding can still reach lived to the end because the program
  said so, and what nothing reaches outlived its own last reference, which
  nothing but a cycle does. It does not follow a `weak` edge, because
  following one would let a cycle report itself as reachable. What is still
  missing is the collector, and why there is none is argued in
  [`docs/memory.md`](docs/memory.md) §5.
* Windows used to be here. It is not: the whole suite runs there on both
  ABIs, including the `x86_64-pc-windows-msvc` binary the release workflow
  ships, and including JNI from an actor thread. `keal build` needs a C
  driver that is not MSVC — the runtime checks overflow with GCC and Clang
  builtins — and [`docs/interop.md`](docs/interop.md) says what to install
  and why. Running the suite there is what turned up the line endings, the
  path separators, an error message written in the operating system's own
  language, and a site generator that deleted a page on its way out.
* Smaller items: native `try` catching C stack exhaustion (the VM's depth
  panic is catchable, a native segfault is not), a register-based VM if the
  bytecode engine ever needs to be faster than it is, and enum variants
  that carry data — refused for now because they would be this language's
  first subtyping relation, and staged so they can arrive as an addition
  rather than a rewrite.

Class inheritance is a **non-goal**: composition, traits with default
methods and records cover the territory without the diamond.

Everything this list was opened with has been built. What is left above is
the honest remainder: one thing reference counting cannot do on its own, and
a handful of conveniences. The next thing to work on is whatever the first
person to write a real program in Keal finds missing — and that is a better
list than one written from here.

## Taking part

* [CONTRIBUTING.md](CONTRIBUTING.md) — the whole procedure: the rules
  every change must respect, and the three commands that verify them.
* [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — everyone is welcome here,
  at every level of experience. Not knowing a word yet is an ordinary
  state to be in, and asking is the right move.
* [SECURITY.md](SECURITY.md) — the threat model stated plainly (compiling
  a Keal program runs its author's code), and how to report privately.

## License

Apache License 2.0 — see [LICENSE](LICENSE). It grants the patent licence
an MIT-style notice leaves unsaid, which is what a compiler wants: code
that goes through Keal comes out the other side as C, and everyone
involved should know where they stand.
