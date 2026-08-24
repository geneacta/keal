# The Keal language

Keal is a small, statically typed, garbage-free-by-construction scripting
language. It is close in spirit to Kotlin, with a C-family surface syntax:
declared types, mandatory braces, classes with primary constructors, and
null safety enforced by the type checker.

Everything below is implemented and covered by the test suite.

---

## 1. Files, statements and semicolons

A file is a sequence of declarations (`fun`, `class`, `import`) and
statements. Statements at the top level run in order; declarations are
visible everywhere in the program, including before the line that declares
them.

```keal
greet()                       // fine: `greet` is declared below

fun greet() { println("hi") }
```

Semicolons are optional. A newline ends a statement whenever the line so far
could be a complete statement, so this works as written:

```keal
val a = 1
val b = 2
val c = a +
        b                     // continues: a line cannot end with `+`
```

The one consequence worth remembering: **an opening brace must sit on the
same line as the construct it belongs to.**

```keal
fun f() {                     // correct
}

fun g()
{                             // error: the newline ended the declaration
}
```

Newlines inside `(...)` and `[...]` never end a statement, so argument lists
and list literals may be spread over several lines freely.

Comments are `// line`, and `/* block */`, which nests.

---

## 2. Values and types

| Type | Values | Notes |
|---|---|---|
| `Int` | `0`, `-7`, `1_000_000` | 64-bit signed; overflow is an error, not a wrap |
| `Float` | `1.5`, `-0.25`, `6.02e23` | 64-bit IEEE 754 |
| `Bool` | `true`, `false` | |
| `String` | `"text"` | immutable, indexed by character |
| `Unit` | — | the type of a statement that produces no value |
| `List<T>` | `[1, 2, 3]` | mutable, ordered |
| `Map<K, V>` | `{"a": 1}` | mutable, insertion-ordered |
| `Range` | `0..10` | half-open: contains 0 through 9 |
| `T?` | `null` or a `T` | see [null safety](#6-null-safety) |
| `(A, B) -> C` | lambdas, functions | first-class function values |
| `Any` | anything non-null | narrow it with `is` before use |
| `Nothing` | — | the type of an expression that never returns |

### Numbers do not convert implicitly

`Int` and `Float` are separate types, and Keal will not silently mix them:

```keal
val n = 3
val bad = n / 2.0            // error: `/` cannot be applied to `Int` and `Float`
val good = n.toFloat() / 2.0 // 1.5
```

The one exception is an integer *literal*, which adapts to a `Float` context.
This is safe because a literal has no other meaning to lose:

```keal
val x: Float = 3             // 3.0
val y: Float = 1 / 2         // 0.5 — the whole literal expression is Float
val z = 2.0 * 3              // 6.0 — the literal 3 adapts to its neighbour
```

---

## 3. Bindings

`val` binds once; `var` may be reassigned. The type is inferred from the
initializer unless you write it down.

```keal
val name = "Ada"             // String
var count = 0                // Int
val ratio: Float = 1         // 1.0, thanks to literal adaptation
count += 1
```

An initializer is always required. Parameters and loop variables are
immutable, like `val`.

---

## 4. Expressions

### Operators, tightest binding last

```
||   &&   ==  !=   <  <=  >  >=   is   in   ?:   ..   +  -   *  /  %   unary - !   . ?. [] ()
```

`&&` and `||` short-circuit. `+` on a `String` appends the rendered form of
whatever is on the right (`"n = " + 3`).

### Blocks are expressions

The value of a block is the value of its last expression. This is why `if`
and `when` can produce values, and why `return` is optional at the end of a
function.

```keal
val size = if (n < 10) { "small" } else { "large" }

val computed = if (ready) {
    val a = 2
    val b = 3
    a * b                    // the block's value
} else {
    0
}

fun double(n: Int): Int { n * 2 }   // no `return` needed
```

Braces are mandatory on `if`, `while` and `for` bodies. An `if` used as a
value must have an `else`.

### `when`

With a subject, `when` compares against it; without one, it is a chain of
conditions. The first matching arm wins.

```keal
when (n) {
    0 -> "zero"
    1, 2, 3 -> "small"       // several values in one arm
    in 4..10 -> "medium"     // a range
    is String -> "text"      // a type test
    else -> "other"
}

when {
    n < 0 -> "negative"
    n == 0 -> "zero"
    else -> "positive"
}
```

An arm's body is a single expression, or a `{ ... }` block. A `when` that
produces a value needs an `else` arm.

### String interpolation

```keal
val name = "Ada"
println("hello ${name}, ${1 + 2} things")
println("short form: $name")
println("a literal dollar: \$5")
```

Escapes: `\n`, `\t`, `\r`, `\0`, `\\`, `\"`, `\$`, and `\u{1F600}`.

### Lambdas

A `{ ... }` in expression position is a lambda. Parameter types are inferred
from the context; a lambda with no parameter list takes one implicit
parameter named `it`.

```keal
val double = { x: Int -> x * 2 }
xs.map({ n -> n + 1 })       // n is inferred as the element type
xs.map({ it * 2 })           // the implicit `it`
val now = { -> time() }      // no parameters
```

A lambda's value is its last expression. `return` inside a lambda is an
error, because it would be ambiguous about which function it leaves.

Lambdas close over variables, not values:

```keal
fun counter(): () -> Int {
    var n = 0
    return { -> n += 1; n }
}
val next = counter()
next()                       // 1
next()                       // 2
```

---

## 5. Statements

```keal
while (cond) { ... }
for (x in xs) { ... }        // over List, Map (keys), String (characters), Range
break
continue
return                       // or `return value`
```

`for` walks a snapshot of the collection, so mutating it inside the loop
cannot invalidate the iteration.

---

## 6. Null safety

A type does not admit `null` unless you write `?`. The checker will not let a
nullable value be used as if it were not.

```keal
var maybe: String? = null
maybe.length                 // error: `String?` may be null
```

Four ways to get from `T?` to `T`:

```keal
maybe?.length                // safe call — the whole expression is Int?
maybe ?: "default"           // elvis — a fallback value
maybe!!                      // assert non-null; fails at run time if it is
if (maybe != null) { maybe.length }   // smart cast
```

### Smart casts

After a check that proves a fact about an **immutable** binding, that fact
holds for the rest of the branch:

```keal
if (s != null) { s.length }             // narrowed inside the branch
if (s == null) { return }
s.length                                // narrowed for the rest of the block
s != null && s.length > 0               // narrowed in the right operand
if (v is String) { v.toUpper() }        // `is` narrows too
```

A `var` is never narrowed — anything the branch calls could reassign it. Copy
it into a `val` first, and the checker will say so.

### `is`

`is` tests a value's outer type at run time. Because type arguments are not
observable at run time, `is List<Int>` is rejected and `is List` accepted:

```keal
fun describe(v: Any): String {
    return when (v) {
        is Int -> "int ${v + 1}"
        is List -> "list of ${v.size}"
        else -> typeOf(v)
    }
}
```

---

## 7. Functions

```keal
fun add(a: Int, b: Int): Int { a + b }

fun greet(name: String, greeting: String = "hello"): String {
    return "${greeting}, ${name}!"
}
```

Parameter types are mandatory; the return type defaults to `Unit`. Defaults
may refer to earlier parameters. Arguments may be passed by name, in any
order, and named arguments must come after positional ones:

```keal
greet("Ada")
greet("Ada", greeting = "hi")
greet(greeting = "hi", name = "Ada")
```

Functions are values, and may be nested:

```keal
fun apply(x: Int, f: (Int) -> Int): Int { f(x) }
apply(5, { it * it })                    // 25

fun outer(base: Int): Int {
    fun inner(k: Int): Int { base + k }  // captures `base`
    return inner(1) + inner(2)
}
```

A function with a declared return type must produce one on every path, either
by `return` or as its last expression.

---

## 8. Classes

```keal
class Point(val x: Float, val y: Float) {
    val magnitude: Float = sqrt(x * x + y * y)   // sees constructor parameters
    var label: String = "?"

    fun plus(other: Point): Point {
        return Point(this.x + other.x, this.y + other.y)
    }

    fun toString(): String { "${this.label}(${this.x}, ${this.y})" }
}

val p = Point(3.0, 4.0)
p.label = "P"
println(p)                  // P(3.0, 4.0)
```

* Constructor parameters marked `val`/`var` become fields; unmarked ones are
  visible only to field initializers.
* Field initializers run top to bottom and may use `this` and the fields
  above them.
* Members are always accessed through `this` inside a method.
* A class with no body needs no braces: `class Pair(val a: Int, val b: String)`.
* Defining `fun toString(): String` changes how `println` and interpolation
  render the instance; otherwise they print `Point(x=3.0, y=4.0)`.
* Instances compare by **identity**. Structural comparison would loop forever
  on a cyclic object graph, so `==` on two separately built instances is
  `false`.
* There is no inheritance, no interfaces and no static members yet.

Classes may only be declared at the top level.

---

## 9. Modules

```keal
import "./geometry.keal"
```

Paths are relative to the importing file. Everything the imported file
declares becomes visible; there is one flat namespace. A file is loaded at
most once, so diamond imports and cycles are both fine.

---

## 10. Standard library

### Free functions

| | |
|---|---|
| `println(value)` `print(value)` | write to standard output |
| `readLine(): String?` | one line of input, `null` at end of file |
| `panic(message): Nothing` | abort with an error |
| `assert(condition, message = ...)` | abort unless the condition holds |
| `typeOf(value): String` | the value's run-time type name |
| `abs(x)` `min(a, b)` `max(a, b)` | `Int` or `Float`, matching the arguments |
| `sqrt(x)` `pow(base, exp)` | `Float` |
| `floor(x)` `ceil(x)` `round(x)` | `Float` to `Int` |
| `random(): Float` | in `[0, 1)` |
| `randomInt(min, max): Int` | in `[min, max)` |
| `time(): Float` | seconds since the Unix epoch |

### `String`

`.length` · `isEmpty` · `substring(start, end)` · `take(n)` · `drop(n)` ·
`get(i)` · `split(sep)` · `trim` · `toUpper` · `toLower` · `reversed` ·
`repeat(n)` · `replace(old, new)` · `contains(s)` · `startsWith(s)` ·
`endsWith(s)` · `indexOf(s)` · `chars()` · `toInt(): Int?` ·
`toFloat(): Float?`

Strings are indexed by character, not byte: `"héllo".length` is 5 and
`"héllo"[1]` is `"é"`. Negative indices count from the end.

### `Int` and `Float`

`Int`: `toFloat` · `abs` · `min(other)` · `max(other)` · `pow(exp)` · `toChar`
`Float`: `toInt` · `floor` · `ceil` · `round` · `abs` · `sqrt` · `min` ·
`max` · `pow` · `isNaN`

`toInt` truncates towards zero.

### `List<T>`

`.size` · `isEmpty` · `add(v)` · `addAll(other)` · `insert(i, v)` ·
`get(i)` · `set(i, v)` · `removeAt(i)` · `clear` · `contains(v)` ·
`indexOf(v)` · `first(): T?` · `last(): T?` · `map(f)` · `flatMap(f)` ·
`filter(f)` · `forEach(f)` · `any(f)` · `all(f)` · `none(f)` · `find(f): T?` ·
`count(f)` · `fold(initial, f)` · `sorted` · `sortedBy(key)` · `reversed` ·
`slice(a, b)` · `take(n)` · `drop(n)` · `join(sep = ", ")` · `sum`

Lists are reference values: assigning one does not copy it. `xs[i]` accepts
negative indices. `sorted` works on `Int`, `Float`, `String` and `Bool`.

### `Map<K, V>`

`.size` · `isEmpty` · `get(k): V?` · `set(k, v)` · `remove(k): V?` ·
`containsKey(k)` · `keys(): List<K>` · `values(): List<V>` · `clear`

`m[k]` reads as `V?`, because the key may be missing. Keys must be `Int`,
`Float`, `String`, `Bool` or `null`. Iteration order is insertion order.

### `Range`

`.start` · `.end` · `contains(n)` · `isEmpty` · `toList()`

### Everything

`toString(): String` — the same rendering `println` uses.

---

## 11. Errors

The checker reports every independent error it can find in one pass, sorted
by source position:

```
error: `String?` may be null, so `.length` is not allowed
  --> src/main.keal:2:14
  |
2 |     return s.length
  |              ^
  = note: use `?.`, `!!`, or check for null first
```

At run time, the failures the type system cannot rule out — division by zero,
an index out of range, `!!` on null, runaway recursion — abort with a message
and a call stack.

---

## 12. What is not here yet

Inheritance and interfaces · user-defined generics · exceptions and
`try`/`catch` · pattern destructuring · operator overloading · a module
namespace (imports are flat) · a bytecode VM (the evaluator walks the AST).
