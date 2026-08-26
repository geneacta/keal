# Keal

A small statically typed language with an AST-walking interpreter, written in
Rust with no dependencies.

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

## Build and run

```sh
cargo build --release

./target/release/keal examples/tour.keal     # run a program
./target/release/keal check src/main.keal    # type-check without running
./target/release/keal repl                   # interactive session
```

`cargo test` runs the whole suite: self-checking programs, every example, and
snapshot tests for the diagnostics.

## What the language has

- **Static types with inference.** `Int`, `Float`, `Bool`, `String`, `Unit`,
  `List<T>`, `Map<K, V>`, `Range`, function types, `Any`, `Nothing`. You
  annotate parameters and fields; everything else is inferred.
- **Null safety.** `T?` is a distinct type. `?.`, `?:` and `!!` get you across,
  and the checker smart-casts an immutable binding after `if (x != null)`,
  after an early-return guard, and across `&&`.
- **Classes and records.** Primary constructors that declare fields, field
  initializers, methods, and a `toString` hook that `println` respects. A
  `record` is the data case: immutable fields and `==` comparing them one by
  one, which is what makes a separate `struct` unnecessary.
- **`fun` and `met`.** A `fun` must declare what it returns; a `met` returns
  nothing. The split means `Unit` and `void` are never written by hand, and
  using a `met`'s result is an error rather than a silent no-op.
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
  `when` and function bodies all produce values without ceremony.
- **`when`.** Values, ranges, type tests, and a subject-less form; type tests
  narrow the subject inside the arm.
- **A standard library** of about ninety built-ins over strings, lists, maps
  and numbers, including `map`/`filter`/`fold` typed generically.
- **Modules.** `import "./other.keal"`, resolved relative to the importing
  file and loaded at most once.
- **Diagnostics** that quote the source, point at the column, and suggest a
  fix — sorted by position, with as many independent errors per run as the
  checker can find.

See [`docs/language.md`](docs/language.md) for the full reference and
[`examples/`](examples/) for working programs, including
[a complete arithmetic evaluator written in Keal](examples/calculator.keal).

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
  interp.rs    the evaluator
  native.rs    runtime implementations of the standard library
  prelude.keal the operator traits, written in Keal and compiled in
  loader.rs    module resolution
  repl.rs      interactive session
  span.rs      source locations and diagnostic rendering
```

The pipeline is source → tokens → AST → checked AST → evaluation. The checker
is the only pass that mutates the tree, and it makes exactly one kind of
change: rewriting an integer literal used in a `Float` context.

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
returns, or a `met`, which returns nothing — nothing in between. So there is
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

## Not there yet

Class inheritance, exceptions, destructuring patterns, operator overloading for
user types, namespaced imports — and the native half of the plan: pointers and
references, `constexpr`, macros, C interop, and LLVM code generation. The
evaluator walks the AST, which is the development mode rather than the
destination.
