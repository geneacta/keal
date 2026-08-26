# Keal

A small statically typed language with a bytecode VM, written in Rust with no
dependencies.

Keal takes its shape from Kotlin — declared types with inference, null safety
in the type system, classes with primary constructors, `when`, lambdas — over
a C-family surface syntax, and is heading towards a native backend: generics
are designed for monomorphisation rather than erasure.

```keal
class Point(val x: Float, val y: Float) {
    fun length(): Float { sqrt(this.x * this.x + this.y * this.y) }
    fun toString(): String { "(${this.x}, ${this.y})" }
}

fun firstLong(points: List<Point>, min: Float): Point? {
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
keal version
```

**Editing Keal?** [`editors/vscode`](editors/vscode) has a VS Code extension:
highlighting, snippets, and the compiler's own errors reported inline. The
grammar is a plain TextMate file, so Sublime, Zed and others can read it too.

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
- **`fun` and `proc`.** A `fun` must declare what it returns; a `proc` returns
  nothing. The split means `Unit` and `void` are never written by hand, and
  using a `proc`'s result is an error rather than a silent no-op.
- **Operator overloading through traits.** `+`, `-`, `*`, `/`, `%`, unary `-`,
  `==` and the four comparisons are wired to prelude traits (`Add`, `Ord`, …).
  The built-in types implement them too, so `fun total<T: Add>(...)` accepts
  `Int` and your own type alike.
- **Generics and traits.** `fun first<T>(...)`, `class Box<T>`, inferred one
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
  `fun divmod(a: Int, b: Int): (Int, Int) { return a / b, a % b }` returns
  several values of different types, taken apart with `val (q, r) = ...`.
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
until shown otherwise. It is also what a compile-time evaluator for `constexpr`
will be built from.

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

**No `void`.** A declaration is either a `fun`, which must say what it
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
each with its own retain, release, equality and rendering. `Map`, default
arguments, capturing a `var`, and nullable *values* like `Int?` do not
compile yet. Anything it cannot compile is **named**, not mis-compiled:

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

## Not there yet

Class inheritance, exceptions, indexing and call operators, namespaced imports — and the native half of the plan: an explicit
memory model (reference counting, decided), pointers and references,
`constexpr`, macros, C interop, and native code generation.
