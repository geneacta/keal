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

A file may begin with `#!/usr/bin/env keal`, which is ignored, so a script can
be made executable and run directly.

---

## 2. Values and types

| Type | Values | Notes |
|---|---|---|
| `Int` | `0`, `-7`, `1_000_000` | 64-bit signed; overflow is an error, not a wrap |
| `Float` | `1.5`, `-0.25`, `6.02e23` | 64-bit IEEE 754 |
| `Bool` | `true`, `false` | |
| `String` | `"text"` | immutable, indexed by character |
| `Unit` | — | what a `proc` produces: nothing. Never written by hand |
| `List<T>` | `[1, 2, 3]` | mutable, ordered |
| `Map<K, V>` | `{"a": 1}` | mutable, insertion-ordered |
| `Range` | `0..10` | half-open: contains 0 through 9 |
| `T?` | `null` or a `T` | see [null safety](#6-null-safety) |
| `(A, B) -> C` | lambdas, functions | first-class function values |
| `Any` | anything non-null | narrow it with `is` before use |
| `Nothing` | — | the type of an expression that never returns |

Inside a method, the receiver is **`this`**. That is the only receiver
keyword: there is no `self` and no `that`, and both remain ordinary
names a program may use for its own bindings — a lambda parameter called
`self`, as actor handlers conventionally have, is a name and not a
keyword.

### Values and references

There is no `int` versus `Integer` in Keal, and nothing to box or unbox. But
the types do divide in two, and the difference shows:

| | Types | Assigning one |
|---|---|---|
| **Values** | `Int`, `Float`, `Bool`, `Unit` | copies it |
| **References** | `String`, `List<T>`, `Map<K, V>`, class and record instances | shares it |

```keal
var a = 1
var b = a
b += 1
// a is still 1

val xs = [1, 2]
val ys = xs
ys.add(3)
// xs is [1, 2, 3] — the same list
```

Strings are references, but immutable, so nothing can observe the sharing.
Records are references too, and also immutable, so the same holds. The
distinction only becomes visible for `List`, `Map`, and a class with a `var`
field — the three things that can change after they are built. When you need
an independent copy, build one: `xs.slice(0, xs.size)`.

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
not and or xor xnor nand nor implies
==  !=   <  <=  >  >=   is   in   ?:   ..   +  -   *  /  %   unary -   . ?. [] ()
```

`+` on a `String` appends the rendered form of whatever is on the right
(`"n = " + 3`).

### The eight logical connectives

Keal has a native operator for each of the eight two-valued connectives. The
recommended spelling is the word; the familiar symbols are accepted aliases
for four of them, and mean exactly the same thing.

| | Written | Alias | True when |
|---|---|---|---|
| NOT | `not a` | `!a` | `a` is false |
| AND | `a and b` | `a && b` | both |
| OR | `a or b` | `a \|\| b` | either |
| XOR | `a xor b` | `a ^ b` | exactly one |
| XNOR | `a xnor b` | — | both or neither |
| NAND | `a nand b` | — | not both |
| NOR | `a nor b` | — | neither |
| IMPLIES | `a implies b` | — | `b` holds whenever `a` does |

All eight are reserved words.

**No relative precedence.** This is the rule that most distinguishes Keal from
its neighbours: **no connective binds tighter than any other.** Two different
ones side by side is a syntax error, and the parentheses are required even in
the case every other language settles silently:

```koda
a or b and c            // error: `or` and `and` need parentheses
(a or b) and c          // fine
a or (b and c)          // fine — and a different value
```

Most languages give `and` precedence over `or` by convention inherited from
arithmetic. With eight connectives in play that convention stops carrying its
weight: nobody reliably knows how `nand` ranks against `implies`. Rather than
invent an order and expect it to be remembered, Keal asks.

Repeating one connective is allowed exactly where it cannot change the
meaning — that is, where the operator is associative:

```koda
a and b and c           // fine
a xor b xor c           // fine
a nand b nand c         // error: `nand` does not chain
a implies b implies c   // error: group it explicitly
```

`nand` and `nor` are not associative: `(a nand b) nand c` and
`a nand (b nand c)` differ. `implies` is right-associative in logic, but that
is a convention Keal declines to assume on your behalf.

Comparison and arithmetic still bind tighter than every connective, so the
ordinary case needs no parentheses at all:

```koda
1 < 2 and 3 > 2         // (1 < 2) and (3 > 2)
```

`not` is unary and binds as tightly as `!` always has, tighter than any binary
connective: `not a and b` is `(not a) and b`.

**Short-circuiting.** `and`, `or`, `nand`, `nor` and `implies` stop as soon as
the left operand settles the answer:

```koda
false nand slow()      // true, slow() never runs
true nor slow()        // false
false implies slow()   // true — an implication with a false premise holds
```

`xor` and `xnor` **always** evaluate both operands: neither can be decided
from one side alone.

**`implies` and null checks.** Because the right operand of `implies` is only
reached when the left one is true, a null check on the left carries across it:

```koda
fun nonEmpty(s: String?): Bool { s != null implies s.length > 0 }
```

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

### `unless`

`unless (c)` is `if (not c)`, and is the same construct in every other
respect: it takes an `else`, it works as an expression, and it narrows types
the same way.

```keal
proc log(s: String, quiet: Bool) {
    unless (quiet) { println(s) }
}

val parity = unless (n % 2 == 0) { "odd" } else { "even" }
```

It reads best as a guard, where `if (not ...)` needs a moment's thought:

```keal
fun lengthOf(s: String?): Int {
    unless (s != null) { return 0 }
    return s.length              // narrowed, exactly as after an `if`
}
```

Chains mix the two freely:

```keal
if (n > 100) {
    "huge"
} else unless (n > 10) {
    "small"
} else {
    "medium"
}
```

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

An arm's body is a single expression, or a `{ ... }` block.

An arm may carry a **guard**: a further condition, judged after the arm's
bindings are in scope.

```keal
when (shape) {
    is Circle(r) if (r > 10.0) -> "huge circle"
    is Circle(r) -> "circle ${r}"
    else -> "something else"
}
```

A `when` that produces a value needs an `else` arm. A guarded arm never
counts as that `else`, because it might not fire — and for the same reason,
what a guarded arm rules out is not assumed by the arms below it.

### Raw strings

`"""..."""` is a raw string: it may span lines, nothing escapes, nothing
interpolates. It is for text meant to be passed through whole — the C inside
a `native` block, a fragment of another language, a block of test input.

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

`is` tests a value's outer type at run time; `!is` is its negation. Only
what survives to run time can be tested — the outer shape — so
`is List<Int>`, `is T` and `is (Int) -> Int` are each rejected with the
reason, while `is List` and a bare class name are accepted:

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

Keal has two declaration words, and which one you use says whether there is a
result:

```keal
fun add(a: Int, b: Int): Int { a + b }      // produces a value, and says which

proc greet(name: String) {                    // produces nothing
    println("hello, ${name}")
}
```

A `fun` **must** declare what it returns. A `proc` **cannot**: it returns
nothing, so there is no `Unit` or `void` to write anywhere in the language.
The rule is enforced both ways — `fun f(n: Int) { ... }` and
`proc f(n: Int): Int { ... }` are each rejected at the declaration.

A `proc` may still `return` early, just with no value:

```keal
proc maybeLog(s: String, keep: Bool) {
    if (not keep) { return }
    println(s)
}
```

The result of a `proc` cannot be used, because there is none:

```keal
val x = greet("Ada")        // error: expression produces no value
println(greet("Ada"))       // error: argument `value` produces no value
```

Everything below is the same for both, so it says `fun` and means either.

```keal
fun greet(name: String, greeting: String = "hello"): String {
    return "${greeting}, ${name}!"
}
```

Parameter types are mandatory. Defaults
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

## 9. Records

A record is a class whose shape already decides its behaviour: every
constructor parameter is a field, all of them immutable, and `==` compares
them one by one.

```keal
record Point(val x: Int, val y: Int)
record Person(name: String, age: Int)      // `val` is implied, and optional
record Empty()

Point(1, 2) == Point(1, 2)                 // true — a class would say false
Point(1, 2).toString()                     // "Point(x=1, y=2)"
```

A record may declare methods, take type parameters and implement traits, like
any other class:

```keal
record Version(val major: Int, val minor: Int) : Ord {
    fun compareTo(other: Version): Int {
        if (this.major != other.major) { return this.major - other.major }
        return this.minor - other.minor
    }
}

Version(2, 0) > Version(1, 9)              // Ord, written by you
Version(1, 0) == Version(1, 0)             // Eq, written for you
```

What a record gives you is exactly one thing: an `Eq` implementation that
compares field by field, and the `Eq` in its trait list. Write your own
`equals` and yours is used instead.

That comparison is safe here in a way it would not be for a class. A record's
fields are immutable and set at construction, so no cycle can be built for the
comparison to fall into — which is why a plain class keeps identity equality
unless you opt in.

Records cannot have `var` fields, in the constructor or in the body. If the
data has to change, use a `class`.

### Destructuring

A binding may name a value's constructor fields instead of the value:

```keal
record Point(val x: Int, val y: Int)

val Point(x, y) = Point(3, 4)     // x is 3, y is 4
val Point(_, only) = p            // `_` skips a field
var Point(mx, my) = p             // `var` makes them assignable
```

The pattern lists the **constructor** fields, in order, and must name all of
them. A field declared in the class body is not positional and takes no part.
Classes with a primary constructor destructure as readily as records.

In a `when` arm, `is T(...)` tests the type and binds in one move:

```keal
fun area(shape: Any): Float {
    return when (shape) {
        is Circle(r) -> 3.14159 * r * r
        is Square(s) -> s * s
        else -> 0.0
    }
}
```

The bindings belong to that arm, so a later arm may reuse the names.

A generic class can be tested but not at particular type arguments — they do
not survive to run time — so `is Pair(a, b)` is accepted, binds `a` and `b`
as `Any`, and `is Pair<Int, Int>(a, b)` is rejected.

### Tuples

Several values of different types travel together as a tuple, without having
to declare a record for them:

```keal
fun divmod(a: Int, b: Int): (Int, Int) {
    return a / b, a % b        // no parentheses needed after `return`
}

val (q, r) = divmod(17, 5)     // q is 3, r is 2
```

`(A, B)` is the type, `(a, b)` the value, and `val (a, b) = t` names its
elements. A tuple holds between two and five values; beyond that, declare a
record, because past five a position stops being memorable and a name starts
earning its keep.

A tuple is an ordinary record underneath, so everything records do it does:

```keal
val pair = (1, "one")
pair.first                     // 1
pair.second                    // "one"
pair == (1, "one")             // true — compared by value
pair.toString()                // "(1, \"one\")"

val pairs: List<(Int, String)> = [(1, "a"), (2, "b")]
```

They nest, though a *pattern* does not: bind the inner value, then destructure
it.

```keal
val nested = ((1, 2), "outer")
val (inner, tag) = nested
val (a, b) = inner
```

Two things tuples deliberately do not replace. A group of values that share a
type is a **list**, `[a, b, c]`. And to take the first of several values that
is present, `?:` already does exactly that, anywhere rather than only in a
`return`:

```keal
return a ?: b ?: c ?: "none"
```

## 10. Generics

Functions and classes may take type parameters. They are written after the
name, as in `fun name<T>(...)` and `class Box<T>`:

```koda
fun firstOr<T>(xs: List<T>, fallback: T): T {
    for (x in xs) { return x }
    return fallback
}

fun mapAll<T, R>(xs: List<T>, f: (T) -> R): List<R> {
    val out: List<R> = []
    for (x in xs) { out.add(f(x)) }
    return out
}

class Box<T>(val value: T) {
    fun get(): T { this.value }
    fun then<R>(f: (T) -> R): Box<R> { Box(f(this.value)) }
}
```

Type arguments are inferred from the call, one argument at a time, so a later
lambda knows the type an earlier argument fixed:

```koda
mapAll([1, 2, 3], { it * 10 })     // T = Int, then R = Int
mapAll(["a", "bb"], { it.length }) // T = String, then R = Int
```

When the arguments cannot settle a parameter, the surrounding annotation can:

```koda
val empty: Stack<Int> = Stack()
```

**Every type parameter must come out concrete.** Keal is meant to compile by
monomorphisation — a generic is emitted once per instantiation — so there is
no boxed representation to fall back on when inference comes up short. Two
consequences follow:

* A generic function cannot be used as a value; it must be called.
* `is T` is rejected: a type parameter has no run-time identity to test.

## 11. Traits

A trait is a named set of method signatures. It is what type-parameter bounds
are written in.

```koda
trait Show {
    fun show(): String
    fun shout(): String { this.show().toUpper() }   // a default body
}

trait Ordered {
    fun compareTo(other: Self): Int
}

class Version(val major: Int, val minor: Int) : Show, Ordered {
    fun show(): String { "v${this.major}.${this.minor}" }
    fun compareTo(other: Version): Int {
        if (this.major != other.major) { return this.major - other.major }
        return this.minor - other.minor
    }
}
```

* A method with a body is a **default**: an implementer inherits it, or
  overrides it by declaring its own.
* `Self` stands for the implementing type. In `Version`, `compareTo` takes a
  `Version`.
* The checker verifies that every required method is present and that its
  signature matches once `Self` is read as the class.

A **bound** makes a trait's methods reachable through a type parameter.
Several bounds are joined with `+`:

```koda
fun describe<T: Show>(value: T): String { value.show() }

fun loudest<T: Show + Ordered>(a: T, b: T): String {
    return if (a.compareTo(b) > 0) { a.shout() } else { b.shout() }
}
```

Without a bound, nothing is known about a type parameter and no method can be
called on it. Bounds apply to classes too: `class Labelled<T: Show>(...)`.

A default method's body is checked in each class that inherits it, not once in
the abstract — the same rule C++ templates follow. A trait nobody implements
is therefore never type-checked.

## 12. Operator overloading

Operators are wired to the traits the prelude declares. A class that
implements one gains the operator; nothing else changes.

| Operator | Trait | Method |
|---|---|---|
| `a + b` | `Add` | `fun plus(other: Self): Self` |
| `a - b` | `Sub` | `fun minus(other: Self): Self` |
| `a * b` | `Mul` | `fun times(other: Self): Self` |
| `a / b` | `Div` | `fun div(other: Self): Self` |
| `a % b` | `Rem` | `fun rem(other: Self): Self` |
| `-a` | `Neg` | `fun negate(): Self` |
| `a == b`, `a != b` | `Eq` | `fun equals(other: Self): Bool` |
| `a < b`, `<=`, `>`, `>=` | `Ord` | `fun compareTo(other: Self): Int` |

```koda
class Vec2(val x: Float, val y: Float) : Add, Neg, Eq {
    fun plus(other: Vec2): Vec2 { Vec2(this.x + other.x, this.y + other.y) }
    fun negate(): Vec2 { Vec2(-this.x, -this.y) }
    fun equals(other: Vec2): Bool { this.x == other.x and this.y == other.y }
}

Vec2(1.0, 2.0) + Vec2(3.0, 4.0)     // Vec2(4.0, 6.0)
```

`a + b` on a class is *rewritten* into `a.plus(b)`, and `a < b` into
`a.compareTo(b) < 0`. The methods stay ordinary methods: you can call
`a.plus(b)` yourself.

**Equality is the exception.** `==` already works on every type by comparing
identity, so a class without `Eq` is not an error — it simply keeps comparing
identity, and two separately built instances are never equal. Implementing
`Eq` is how a class opts into structural equality.

**The built-in types implement these traits too.** `Int` and `Float` provide
all of them, `String` provides `Add`, `Eq` and `Ord`, and `Bool` provides
`Eq`. So a bound is satisfied by a built-in as readily as by your own type:

```koda
fun total<T: Add>(xs: List<T>, zero: T): T {
    var acc = zero
    for (x in xs) { acc = acc + x }
    return acc
}

total([1, 2, 3], 0)                      // 6
total(["a", "b"], "")                    // "ab"
total([Vec2(1.0, 1.0)], Vec2(0.0, 0.0))  // Vec2(1.0, 1.0)
```

Only a class or a bounded type parameter goes through a method call. `1 + 2`
is added directly; the built-in implementations exist so that generic code —
which *is* rewritten — has something to land on.

## 13. Modules and visibility

```keal
import "./geometry.keal"
```

Paths are relative to the importing file. A file is loaded at most once, so
diamond imports and cycles are both fine, and what an import brings in is
one flat namespace — there is no `geometry.` prefix yet.

What it brings in is what the imported file **let** it bring in. A
declaration that says nothing about who may name it is private to its own
file:

```keal
fun rounded(x: Float): Int { ... }        // this file's business
package fun parse(src: String): Ast { ... }  // the files beside it
public class Ast(val root: Node) { ... }     // anyone who imports it
```

| Written | Who may name it |
|---|---|
| nothing, or `private` | the file that declares it |
| `package` | every file in the same directory |
| `public` | every file that imports it |

A **package is a directory**. Nothing declares it and nothing names it: the
files that sit together are the ones that can see each other's `package`
declarations, which is what lets a group of files collaborate without
promising anything to the outside.

The modifier goes on a top-level `fun`, `proc`, `class`, `record`, `trait`,
`extern fun`, `val` or `var`. It is contextual, like `record` and `weak`: a
program that already has a variable called `public` keeps working, because
the word is only a modifier where a declaration follows it.

Inside a body, nothing takes a modifier — a local is reachable exactly where
it is in scope, which is what a scope already says. And a type parameter
named like a class is still a type parameter: `record Pair<A, B>` does not
reach for a class called `A`.

Two consequences worth stating. A private class cannot be *named* either, so
a public function must not return one — the checker refuses the type where
it is written, not later. And the prelude and `lib/jvm.keal` say `public` on
everything, because a standard library is nothing but its public surface.

---

## 14. Standard library

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

## 15. Errors

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

## 16. How a program runs

Keal compiles to bytecode and runs it on a virtual machine. That is an
implementation detail with one visible consequence: `keal --ast file.keal`
runs the same program on the tree-walking evaluator instead, and the two are
required to agree on every byte they print. If they ever do not, that is a
bug — please report it with the program that shows it.

## 17. Calling C and C++

A `native` block passes text verbatim into the C that `keal build` generates:
headers, inline helper functions, declarations. `extern fun` then binds a
symbol with a signature the checker enforces at every call:

```keal
native """
#include <math.h>
static int64_t triple(int64_t n) { return n * 3; }
"""

extern fun sin(x: Float): Float
extern fun triple(n: Int): Int
extern fun pow(base: Float, exponent: Float): Float = "pow"
```

The `= "symbol"` names the C symbol when it differs from the Keal name.

Three rules, each there for a reason:

* **Only `Int`, `Float` and `Bool` cross.** They carry no ownership, so
  neither side has to guess who frees what — the boundary `docs/memory.md`
  section 6 drew. Strings and objects will need an explicit ownership story
  before they cross; refusing now beats leaking later.
* **Every symbol must be declared** — by an included header or by the
  `native` block itself. Keal does not guess prototypes, because a guessed
  prototype that disagrees with a header is exactly the kind of quiet
  miscompilation this backend refuses on principle.
* **Extern functions run natively only.** The interpreters refuse a call by
  name and point at `keal build`.

C++ goes in its own files, behind `extern "C"`:

```cpp
// helpers.cpp — C++ freely inside, C linkage at the boundary
extern "C" int64_t fib_cpp(int64_t n) { /* std::vector, whatever */ }
```

```sh
keal build program.keal helpers.cpp     # links with c++ automatically
```

The generated core stays C11 either way; only the link changes.

## 18. Editor support

[`editors/vscode`](../editors/vscode) holds a Visual Studio Code extension:
highlighting, bracket and indent behaviour, snippets, and a problem matcher
that reads `keal check` output so diagnostics land on the right line.

There is no language server yet, so there is no completion or go-to-definition.
The grammar is a standard TextMate file that Sublime Text, Zed and others read
directly.

## 19. How values are represented

`Int`, `Float`, `Bool`, `Unit` and `Range` are values and copy on assignment.
`String`, `List`, `Map`, functions and instances are references, shared and
reference-counted.

That is all a program needs to know. If you are interested in the bytes —
sizes, field offsets, what `T?` costs, what crosses into C — run
`keal layout file.keal`, and see [`docs/memory.md`](memory.md).

## 19½. `weak` fields

A field may be declared `weak`, before `val` or `var`. It points at its
target without keeping it alive, which is how a cycle's back edge is
written — counting alone can never free a cycle, and since `deinit`
exists a cycle also silently skips its destructors.

```keal
class Item(val id: Int) {
    weak var owner: Owner? = null    // points back, does not hold on
}
class Owner(val id: Int) {
    var held: Item? = null           // holds
}
```

* The type must be `T?` where `T` is a class: a weak reference has to be
  able to read back null.
* Reading gives the target while it lives, `null` from the moment its
  last strong reference dies. Writing never retains.
* A class with a weak field cannot be `copy`-ed, and so cannot cross into
  an actor: an address is not a value to duplicate.
* `weak` is contextual — it is still an ordinary name anywhere else.

The checker cautions where a class declares `deinit` **and** a mutable
field can point straight back at its own object, suggesting `weak` on
the back edge. `docs/memory.md` §5 has the reasoning, the costs and why
there is no cycle collector.

## 20. What is not here yet

Class inheritance (a non-goal) · indexing and call operators (`Index`,
`Invoke`) · associated types on traits · a module namespace (imports are
flat) · typed exceptions (`catch` binds the message as a `String`) ·
`constexpr` and macros · a package manager.

Shipped since this list was first written, and no longer on it: `throw` /
`try` / `catch` on all three engines, destructuring a record in a `when`
or a binding, the native backend through C11 (with C, C++, Rust, Go, Java
and Kotlin interop), actors on real OS threads, `deinit`, and `Any`
natively. What the C backend still refuses, it refuses **by name** —
`keal build` never mis-compiles.
