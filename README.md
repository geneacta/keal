# Keal

A small statically typed language with an AST-walking interpreter, written in
Rust with no dependencies.

Keal takes its shape from Kotlin — declared types with inference, null safety
in the type system, classes with primary constructors, `when`, lambdas — over
a C-family surface syntax.

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
- **Classes.** Primary constructors that declare fields, field initializers,
  methods, and a `toString` hook that `println` respects.
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
  types.rs     the type lattice: assignability, joins, nullability
  builtins.rs  type signatures for the standard library
  checker.rs   name resolution, type checking, null-safety analysis
  value.rs     runtime values, environments
  interp.rs    the evaluator
  native.rs    runtime implementations of the standard library
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

**Built-in generics without a generic system.** `List<T>.map` needs the type
of its lambda's result to type its own. Rather than build inference for
user-facing generics, the checker re-derives a built-in method's signature
after each argument it checks, so `xs.fold(0, { acc, n -> acc + n })` types
the accumulator from the initial value.

## Not there yet

Inheritance and interfaces, user-defined generics, exceptions, destructuring,
operator overloading, namespaced imports, and a bytecode VM. The evaluator
walks the AST; it is fast enough to be pleasant and slow enough that a VM
would be the obvious next project.
