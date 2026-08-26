# A tour of Keal

This is a guided walk through the language, written to be read top to bottom
in about half an hour. Every snippet runs; if one does not, that is a bug.

For the complete rules, see [`docs/language.md`](docs/language.md). For the
short version, see the [README](README.md).

---

## 0. Getting it running

```sh
git clone https://github.com/geneacta/keal.git
cd keal
cargo build --release
```

That leaves a binary at `./target/release/keal`. To have it on your path:

```sh
cargo install --path .
keal version
```

Write a file:

```keal
// hello.keal
println("hello, world")
```

and run it:

```sh
keal hello.keal
```

A script can name its own interpreter, so it can be run directly:

```keal
#!/usr/bin/env keal
println("hello from a script")
```

```sh
chmod +x hello.keal
./hello.keal
```

Two other commands are worth knowing now. `keal check file.keal` type-checks
without running, which is what you want in an editor or a hook. And `keal`
with no arguments opens a REPL:

```
$ keal
keal 0.1.0 — type `:help` for commands, Ctrl-D to leave
>>> 1 + 2
3
>>> val xs = [3, 1, 2]
>>> xs.sorted()
[1, 2, 3]
```

Everything below can be pasted into the REPL as you read.

---

## 1. Values and bindings

`val` binds once, `var` may be reassigned. The type is inferred unless you
write it down.

```keal
val name = "Ada"
var count = 0
count += 1
```

Keal has `Int`, `Float`, `Bool`, `String`, `List<T>`, `Map<K, V>` and ranges.
Numbers do **not** convert implicitly:

```keal
val n = 3
// val bad = n / 2.0        // error: `/` on `Int` and `Float`
val good = n.toFloat() / 2.0 // 1.5
```

The exception is an integer *literal*, which adapts to a `Float` context —
safe because a literal has no other meaning to lose:

```keal
val ratio: Float = 1 / 2     // 0.5, not 0
```

Strings interpolate, and are indexed by character rather than byte:

```keal
val who = "Ada"
println("hello ${who}, ${1 + 2} things")
println("héllo".length)      // 5
println("héllo"[1])          // é
println("abc"[-1])           // c, counting from the end
```

---

## 2. `fun` and `proc`

Two declaration words, and which one you use says whether there is a result.

```keal
fun add(a: Int, b: Int): Int { a + b }   // returns, and says what

proc greet(name: String) {                // returns nothing
    println("hello, ${name}")
}
```

A `fun` **must** declare its return type. A `proc` **cannot** — so `Unit` and
`void` are never written anywhere in a Keal program, and using a `proc`'s
non-result is an error rather than a silent no-op:

```keal
// val x = greet("Ada")     // error: expression produces no value
```

A `proc` can still leave early:

```keal
proc log(s: String, quiet: Bool) {
    if (quiet) { return }
    println(s)
}
```

The last expression of a body is its value, so `return` is often optional.
Parameters can have defaults, and arguments can be passed by name:

```keal
fun greet(name: String, greeting: String = "hello"): String {
    return "${greeting}, ${name}!"
}

greet("Ada")                       // "hello, Ada!"
greet("Ada", greeting = "hi")      // "hi, Ada!"
greet(greeting = "hi", name = "Ada")
```

---

## 3. Control flow, and blocks as values

Braces are mandatory. A block's value is its last expression, which is why
`if` can produce one:

```keal
val sign = if (n < 0) { "neg" } else { "pos" }

val computed = if (ready) {
    val a = 2
    val b = 3
    a * b                 // the block's value
} else {
    0
}
```

`unless (c)` is `if (not c)` — the same construct, with `else` branches and
everything else. It reads best as a guard:

```keal
fun lengthOf(s: String?): Int {
    unless (s != null) { return 0 }
    return s.length
}
```

The two chain freely: `if / else unless / else`, `unless / else if / else`,
and so on.

Loops are what you expect. `for` walks lists, maps (by key), strings (by
character) and ranges, which are half-open:

```keal
for (i in 0..3) { println(i) }        // 0 1 2
for (c in "abc") { println(c) }
for (name in ages) { println(name) }  // a map yields its keys

var i = 0
while (i < 3) { i += 1 }
```

---

## 4. `when`: the switch and the match

`when` is Keal's only branching-on-shape construct, and it covers what other
languages split between `switch` and `match`. There is no fall-through, the
first matching arm wins, and it is an expression.

With a subject, arms compare against it:

```keal
fun describe(n: Int): String {
    return when (n) {
        0 -> "zero"
        1, 2, 3 -> "small"        // several values in one arm
        in 4..10 -> "medium"      // a range
        else -> "large"
    }
}
```

Without a subject, it is a table of conditions — a chain of `if` without the
staircase:

```keal
when {
    n < 0 -> "negative"
    n == 0 -> "zero"
    else -> "positive"
}
```

`is` tests the type, and narrows the subject inside the arm, so the value is
usable at that type without a cast:

```keal
fun render(v: Any): String {
    return when (v) {
        is Int -> "int ${v + 1}"          // v is an Int here
        is String -> v.toUpper()          // and a String here
        is List -> "list of ${v.size}"
        else -> typeOf(v)
    }
}
```

An arm may carry a **guard**, an extra condition judged after its bindings are
in scope:

```keal
when (shape) {
    is Circle(r) if (r > 10.0) -> "huge circle"
    is Circle(r) -> "circle ${r}"
    else -> "something else"
}
```

A `when` that produces a value must have an `else`. A guarded arm never counts
as that `else`, because it might not fire.

---

## 5. Null safety

A type does not admit `null` unless you write `?`, and the checker will not
let you forget:

```keal
var maybe: String? = null
// maybe.length            // error: `String?` may be null
```

Four ways across:

```keal
maybe?.length              // safe call — the result is Int?
maybe ?: "default"         // a fallback
maybe!!                    // assert non-null; fails at run time if it is
if (maybe != null) { maybe.length }   // smart cast
```

That last one is the interesting one. After a check that proves something
about an **immutable** binding, the fact holds for the rest of the branch —
and Keal carries it further than most:

```keal
if (s == null) { return }
s.length                              // narrowed for the rest of the block

s != null and s.length > 0            // narrowed in the right operand

s != null implies s.length > 0        // and across `implies`

when {
    s == null -> "none"
    else -> "${s.length}"             // the else knows s is not null
}
```

A `var` is never narrowed — anything the branch calls could reassign it — and
the checker says so rather than leaving you to wonder.

---

## 6. Collections and lambdas

```keal
val xs = [3, 1, 4, 1, 5]
val ages = {"ada": 36, "grace": 45}

xs.size                    // 5
xs[0]                      // 3
xs[-1]                     // 5, from the end
ages["ada"]                // 36, typed Int? because the key may be missing
ages["nobody"] ?: 0        // 0
```

Lambdas are `{ ... }` in expression position. Parameter types come from the
context, and a lambda with no parameter list takes one named `it`:

```keal
xs.map({ it * 2 })                       // [6, 2, 8, 2, 10]
xs.filter({ n -> n > 3 })                // [4, 5]
xs.fold(0, { acc, n -> acc + n })         // 14
xs.sorted().join(" ")                     // "1 1 3 4 5"
```

Closures capture variables, not values:

```keal
fun counter(): () -> Int {
    var n = 0
    return { -> n += 1; n }
}
val next = counter()
next()    // 1
next()    // 2
```

---

## 7. Records and classes

A **record** is data: every constructor parameter is a field, all immutable,
and `==` compares them one by one.

```keal
record Point(val x: Int, val y: Int)
record Person(name: String, age: Int)   // `val` is implied, and optional

Point(1, 2) == Point(1, 2)              // true
Point(1, 2).toString()                  // "Point(x=1, y=2)"
```

A **class** is for behaviour and mutable state. Without an `Eq`
implementation it keeps identity equality, so two separately built instances
are never equal:

```keal
class Counter {
    var n: Int = 0
    proc bump() { this.n += 1 }
    fun value(): Int { this.n }
}

class Point3(val x: Float, val y: Float) {
    val length: Float = sqrt(x * x + y * y)   // sees the constructor params
    fun toString(): String { "(${this.x}, ${this.y})" }
}
```

Defining `toString` changes what `println` and interpolation show.

Members are always reached through `this` inside a method. There is no
inheritance — traits and composition do that job.

### Destructuring

A binding can name the constructor fields instead of the value:

```keal
val Point(x, y) = p          // x is 1, y is 2
val Point(_, only) = p       // `_` skips one
```

And in a `when` arm, `is T(...)` tests and binds in one move:

```keal
when (shape) {
    is Circle(r) -> 3.14159 * r * r
    is Square(s) -> s * s
    else -> 0.0
}
```

---

## 8. Generics and traits

Type parameters go after the name, on functions and on classes alike:

```keal
fun firstOr<T>(xs: List<T>, fallback: T): T {
    for (x in xs) { return x }
    return fallback
}

class Box<T>(val value: T) {
    fun get(): T { this.value }
    fun then<R>(f: (T) -> R): Box<R> { Box(f(this.value)) }
}
```

Type arguments are inferred one argument at a time, so a later lambda knows
what an earlier argument fixed:

```keal
mapAll([1, 2, 3], { it * 10 })     // T from the list, then R from the body
```

A **trait** is a set of method signatures, and what bounds are written in:

```keal
trait Show {
    fun show(): String
    fun shout(): String { this.show().toUpper() }   // a default
}

class Tag(val name: String) : Show {
    fun show(): String { "#${this.name}" }
}

fun describe<T: Show>(value: T): String { value.show() }
```

`Self` in a trait stands for the implementing type. Several bounds join with
`+`: `<T: Show + Ordered>`.

### Operators come from traits

`+`, `-`, `*`, `/`, `%`, unary `-`, `==` and the four comparisons are wired to
traits the prelude declares. Implement one and your type gains the operator:

```keal
class Vec2(val x: Float, val y: Float) : Add, Neg, Eq {
    fun plus(other: Vec2): Vec2 { Vec2(this.x + other.x, this.y + other.y) }
    fun negate(): Vec2 { Vec2(-this.x, -this.y) }
    fun equals(other: Vec2): Bool { this.x == other.x and this.y == other.y }
}

Vec2(1.0, 2.0) + Vec2(3.0, 4.0)
```

The built-in types implement the same traits, so a bound accepts `Int` as
readily as your own type:

```keal
fun total<T: Add>(xs: List<T>, zero: T): T {
    var acc = zero
    for (x in xs) { acc = acc + x }
    return acc
}

total([1, 2, 3], 0)        // 6
total(["a", "b"], "")      // "ab"
```

---

## 9. The eight logical connectives

Keal has a native operator for each, spelled as a word. `!`, `&&`, `||` and
`^` are accepted aliases for four of them.

```keal
not a        a and b       a or b        a xor b
a xnor b     a nand b      a nor b       a implies b
```

**None binds tighter than another.** `a or b and c` is a syntax error; the
parentheses are required:

```keal
(a or b) and c
a or (b and c)     // a different value
```

Most languages rank `and` above `or` by a convention borrowed from
arithmetic. With eight connectives that convention stops carrying its weight —
nobody reliably knows how `nand` ranks against `implies` — so Keal asks rather
than guessing. Repeating one operator is still fine where it cannot change the
meaning, so `a and b and c` works and `a nand b nand c` does not.

Comparison and arithmetic bind tighter than every connective, so the ordinary
case needs no parentheses: `1 < 2 and 3 > 2`.

`and`, `or`, `nand`, `nor` and `implies` short-circuit. `xor` and `xnor`
cannot, and always evaluate both sides.

---

## 10. Several files

```keal
import "./geometry.keal"
```

Paths are relative to the importing file, everything the imported file
declares becomes visible, and a file is loaded at most once — so diamonds and
cycles are both fine.

---

## 11. When something goes wrong

The checker reports every independent error it finds, sorted by position,
quoting the line and pointing at the column:

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

## Where to go next

* [`docs/language.md`](docs/language.md) — the complete reference.
* [`examples/`](examples/) — working programs, including
  [an arithmetic evaluator written in Keal](examples/calculator.keal):
  tokenizer, precedence parser and evaluation in about 120 lines.
* [`tests/programs/`](tests/programs/) — every feature, exercised by
  assertions. These are the most precise description of what the language
  does, because they fail when it stops doing it.
