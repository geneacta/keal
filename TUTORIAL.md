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
without running, which is what you want in an editor or a hook — and when the
program is ready, `keal build file.keal` compiles it to a native executable
(section 11½). And `keal`
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

Assigning a number copies it; assigning a list or a map shares it:

```keal
var a = 1
var b = a
b += 1                       // a is still 1

val xs = [1, 2]
val ys = xs
ys.add(3)                    // xs is [1, 2, 3] — the same list
```

That is the whole of the value/reference distinction. `String` and records
are shared too, but they are immutable, so nothing can tell. Only `List`,
`Map` and classes with a `var` field can change after they are built, which
is where sharing becomes visible.

Strings interpolate, and are indexed by character rather than byte:

```keal
val who = "Ada"
println("hello ${who}, ${1 + 2} things")
println("héllo".length)      // 5
println("héllo"[1])          // é
println("abc"[-1])           // c, counting from the end
```

---

## 2. `func` and `proc`

Two declaration words, and which one you use says whether there is a result.

```keal
func add(a: Int, b: Int): Int { a + b }   // returns, and says what

proc greet(name: String) {                // returns nothing
    println("hello, ${name}")
}
```

A `func` **must** declare its return type. A `proc` **cannot** — so `Unit` and
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
func greet(name: String, greeting: String = "hello"): String {
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
func lengthOf(s: String?): Int {
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
func describe(n: Int): String {
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
func render(v: Any): String {
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
func counter(): () -> Int {
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
    func value(): Int { this.n }
}

class Point3(val x: Float, val y: Float) {
    val length: Float = sqrt(x * x + y * y)   // sees the constructor params
    func toString(): String { "(${this.x}, ${this.y})" }
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

### Tuples

To return several values of different types, group them:

```keal
func divmod(a: Int, b: Int): (Int, Int) {
    return a / b, a % b
}

val (q, r) = divmod(17, 5)     // q is 3, r is 2
```

`(A, B)` is the type and `(a, b)` the value. A tuple is a record underneath,
so it compares by value, renders as `(1, "one")`, and its elements can be
reached by name — `pair.first`, `pair.second`. Two to five values; beyond
that, declare a record.

For several values that *share* a type, use a list. For the first of several
that is present, `?:` already does it.

---

## 8. Generics and traits

Type parameters go after the name, on functions and on classes alike:

```keal
func firstOr<T>(xs: List<T>, fallback: T): T {
    for (x in xs) { return x }
    return fallback
}

class Box<T>(val value: T) {
    func get(): T { this.value }
    func then<R>(f: (T) -> R): Box<R> { Box(f(this.value)) }
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
    func show(): String
    func shout(): String { this.show().toUpper() }   // a default
}

class Tag(val name: String) : Show {
    func show(): String { "#${this.name}" }
}

func describe<T: Show>(value: T): String { value.show() }
```

`Self` in a trait stands for the implementing type. Several bounds join with
`+`: `<T: Show + Ordered>`.

### Operators come from traits

`+`, `-`, `*`, `/`, `%`, unary `-`, `==` and the four comparisons are wired to
traits the prelude declares. Implement one and your type gains the operator:

```keal
class Vec2(val x: Float, val y: Float) : Add, Neg, Eq {
    func plus(other: Vec2): Vec2 { Vec2(this.x + other.x, this.y + other.y) }
    func negate(): Vec2 { Vec2(-this.x, -this.y) }
    func equals(other: Vec2): Bool { this.x == other.x and this.y == other.y }
}

Vec2(1.0, 2.0) + Vec2(3.0, 4.0)
```

The built-in types implement the same traits, so a bound accepts `Int` as
readily as your own type:

```keal
func total<T: Add>(xs: List<T>, zero: T): T {
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

Paths are relative to the importing file, and a file is loaded at most once —
so diamonds and cycles are both fine. What the import brings in is one flat
namespace, and what it brings in at all is what the other file allowed:

```keal
func rounded(x: Float): Int { ... }           // this file only
package func parse(src: String): Ast { ... }  // the files beside it
public class Ast(val root: Node) { ... }     // whoever imports it
```

A declaration that says nothing is private to its own file. `package` opens
it to every file in the same directory — a package is a directory, nothing
declares it — and `public` opens it to anyone who imports the file. The
words are contextual, so a program that already calls something `public`
keeps working.

Write the modifier on a top-level `func`, `proc`, `class`, `record`, `trait`,
`extern func`, `val` or `var` — and on a class's members, where the same
default applies:

```keal
public class Counter(public var n: Int) {
    var steps: Int = 0                 // the class's own business
    public proc bump() { this.n += 1 }
}
```

A record is different, because a record *is* its fields: they follow the
record's own visibility unless one of them says otherwise. Inside a body
there is nothing to write a modifier on — a local is reachable exactly where
it is in scope.

Two modules may declare the same name. Give one of them an alias and say
which you mean:

```keal
import "./lexer.keal"                 // its names, bare
import "./config.keal" as config      // its names, through `config`

val token = parse("let")              // the lexer's
val setting = config.parse("width")   // the other one's
val n: config.Node = setting          // its type, too
```

Writing a name that both declare is an error where you write it, not where
you import — so two modules sharing a name never break a program that does
not mention it.

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

## 11½. Compiling to native code, and calling C

Everything above runs on the bytecode VM. The same program compiles to a real
executable:

```sh
$ keal build hello.keal
hello
$ ./hello
```

On numeric work that is roughly 80× the VM. The generated code keeps the
language's guarantees — integer overflow still fails, bounds are still
checked, and the test suite requires native output to match the interpreters
byte for byte.

Native code can also call C, and C++ behind `extern "C"`:

```keal
native """
#include <math.h>
"""

extern func sin(x: Float): Float

println(sin(0.0))
```

`native` passes text into the generated C verbatim; `extern func` binds a
symbol with a checked signature. Only `Int`, `Float` and `Bool` cross the
boundary — they carry no ownership, so neither side has to guess who frees
what. With C++ in a separate file:

```sh
$ keal build program.keal helpers.cpp
```

The triple-quoted string used above is a **raw string**: newlines welcome, no
escapes, no interpolation — for text meant to pass through whole.

Strings and small records cross the boundary too. A `String` must say who
owns it — `borrow String` going in, `own String` coming back — and a record
of `Int`/`Float`/`Bool` fields passes by copy as the mirror struct
`Keal_Name` the generated C defines. And the door swings both ways: every
Keal function with a clean signature is a C symbol `k_name`, and
`keal emit-header program.keal` prints the header a companion `.c` file
needs to call back in.

---

## Power, root, and spoken comparisons

`**` is power (right-associative, checked on `Int`), and `^/` is its
inverse, the root — `//` was taken by comments, so the root wears the
power's hat: `27 ^/ 3` is 3, `x ^/= 2` takes a square root in place.
`x++` and `x--` are statements, sugar for `x += 1` and `x -= 1`.

```keal
assert(2 ** 3 ** 2 == 512, "powers associate to the right")
assert(26 ^/ 3 == 2, "integer roots floor")
var acc = 3
acc **= 2
acc++
assert(acc == 10, "compounds and increments")
```

Comparisons can speak, too: `a <=> b` — the spaceship — works on any
`Ord` type and answers a `Comp`: `less`, `equal` or `greater`,
three-valued the way `Bool` is two-valued (`compare(a, b)` is the same
thing as a function). The ternary understands both: two branches select
on a `Bool`, three select on a `Comp` — lazily, the condition evaluated
exactly once:

```keal
val verdict = 3 < 5 ? "small" : "big"
assert(verdict == "small", "the plain ternary")
val word = "kiwi" <=> "fig" ? "before" : "same" : "after"
assert(word == "after", "the three-way ternary")
```

And a `return` can carry its own guard:

```keal
func cheapest<T: Ord>(a: T, b: T): T {
    return if (compare(a, b).isAtMost()) a
    return b
}
assert(cheapest(4, 9) == 4, "guarded return, spoken comparison")
```

## Throwing and catching

Every failure in Keal is a panic with a message — overflow, division by
zero, a failed `assert`. `throw "message"` raises your own, and
`try`/`catch` intercepts any of them, binding the message:

```keal
func parseAge(s: String): Int {
    val n = s.toInt()
    if (n == null) { throw "not a number: ${s}" }
    return if (n >= 0) n
    throw "an age cannot be negative: ${n}"
}

var caught = ""
try {
    parseAge("-3")
} catch (e) {
    caught = e
}
assert(caught == "an age cannot be negative: -3", "caught our own throw")

try {
    val zero = 0
    println(10 / zero)
} catch (e) {
    assert(e == "division by zero", "built-in panics are the same kind")
}
```

`return`, `break` and `continue` pass through a `try` untouched — only
panics are caught. This works identically on all three engines — a native
binary unwinds with zero leaks, and even a Java exception arriving through
the JVM gateway lands in your `catch`.

## One file, six languages

Everything the interop chapters promise, in one program
([`examples/interop/polyglot/`](examples/interop/polyglot/), run
`./run.sh` there):

```keal
import "./bindings.keal"     // C + C++ + Rust + Go: one bindgen header
import java.util.UUID        // Java: generated wrappers, no path
import GreeterKt             // Kotlin: same JVM road

println(c_add(40, 2))                  // C, a source file on the build line
println(cpp_shout("c plus plus"))      // C++ behind extern "C"
println(rust_fib(20))                  // Rust, a staticlib
println(rust_greet("Keal"))
println(go_hypot(3.0, 4.0))            // Go, a c-archive
println(go_shout("go"))

jvmStart("-Djava.class.path=kotlin/greeter.jar")
val id = uuidFromString("123e4567-e89b-12d3-a456-426614174000")
println(id.toString())                 // Java, wrapped by `keal jbind`
println(id.version())
println(greeterKtShout("kotlin"))      // Kotlin, plain JVM classes
println(greeterKtFib(20))
id.free()
```

The native four meet Keal at the C ABI its binaries already speak — no
runtime, no marshalling layer — and the JVM two go through one gateway
module written in Keal itself. (This section has no mirrored test:
it needs six toolchains. The example directory is the proof.)

## `deinit`: when the last reference dies

A class may declare `proc deinit()`. It runs when the object's last
reference dies — at the next statement boundary, exactly once, youngest
object first:

```keal
var closed = 0
class Session(val id: Int) {
    proc deinit() { closed += 1 }
}
if (true) {
    val s = Session(7)
    assert(closed == 0, "alive while in scope")
}
assert(closed == 1, "deinit ran when the block ended")
```

Calling `deinit` yourself is a checker error — it is the runtime's to
call. The generated JVM wrappers use it to free their handles, so a
forgotten `free()` no longer leaks a global reference.

## `weak`: the back edge that does not hold on

Counting references has one blind spot: a **cycle**. If `a` holds `b`
and `b` holds `a`, neither count ever reaches zero, so neither object is
freed — and, worse, neither runs its `deinit`. Silently.

`weak` breaks the loop. Written before `val` or `var` on a class field,
it says *this field points at something without keeping it alive*:

```keal
class Item(val id: Int) {
    weak var owner: Owner? = null      // points back, does not hold on
}
class Owner(val id: Int) {
    var held: Item? = null             // holds
}

val o = Owner(1)
val it = Item(2)
o.held = it
it.owner = o        // a loop on paper; not a loop in the counts
```

When `o` and `it` go out of scope, both die and both destructors run.
The strong edge is the one that owns; the weak one is just an address.

Reading a weak field gives you the target **while it lives**, and `null`
the moment its last strong reference dies — which is why the type has to
be nullable:

```keal
class Watcher(val name: String) {
    weak var watched: Leaf? = null
}
val w = Watcher("w")
if (true) {
    val leaf = Leaf(9)
    w.watched = leaf
    assert(w.watched != null, "alive inside the block")
}
assert(w.watched == null, "and gone after it")
```

Three rules the checker holds you to, each with its reason:

* **`weak` needs a `T?` where `T` is a class.** A weak reference has to
  be able to read back null, and only a counted object can die while
  someone still names it. `weak var n: Int?` is refused.
* **A class with a weak field does not `copy`.** An address is not a
  value to duplicate — and since actors use the same rule, a weak field
  cannot travel in a message either.
* **You are cautioned about the shape that bites.** A class that
  declares `deinit` and has a mutable field able to point straight back
  at its own object gets a warning, with `weak` as the suggested fix.
  It is a caution, not an error: you may know the graph never closes.

Everything else is unchanged. A program that never writes `weak` pays
nothing for it — natively, objects carry the same single count they
always did.

## Lazy sequences

The prelude carries a pull-based pipeline — Keal's `Stream`/`Sequence` —
written in ordinary Keal. Nothing runs until a terminal operation pulls:

```keal
var calls = 0
val two = seq([10, 20, 30, 40]).map({ n ->
    calls += 1
    n + 1
}).take(2).toList()
assert(two == [11, 21], "only the taken elements were computed")
assert(calls == 2, "take(2) pulled exactly twice")
```

Infinite sources are fine as long as something downstream stops:

```keal
val powers = iterate(1, { it * 2 }).take(5).toList()
assert(powers == [1, 2, 4, 8, 16], "iterate is lazy")
```

`map`, `filter`, `take`, `drop`, `takeWhile`, `dropWhile` and `flatMap` are
the lazy stages; `toList`, `forEach`, `fold`, `count`, `any`, `all` and
`first` pull. Everything compiles to native like the rest of the language.

## Actors

Concurrency in Keal is actors: state owned by one handler, messages
between them, and — today — a deterministic round-robin scheduler, so a
program computes the same thing on every engine. The ownership rules are
real: **each actor gets its own copy of every capture** (its state is its
own), **messages cross by copy**, and results leave through an `Outbox` —
an address, shared like an `ActorRef`:

```keal
proc tallyDown(steps: Outbox<Int>) {
    val tally: ActorSystem<Int> = ActorSystem()
    var total = 0
    val counter = tally.spawn({ self, n ->
        total += n
        steps.post(total)
        if (n > 1) { self.send(n / 2) }
    })
    counter.send(8)
    tally.run()
    assert(total == 0, "the actor accumulated in its own copy, not mine")
}
val steps: Outbox<Int> = outbox()
tallyDown(steps)
assert(steps.drain() == [8, 12, 14, 15], "8, then self-sent 4, 2, 1")
```

A word on that first parameter. Keal's receiver keyword is **`this`**,
and only `this` — there is no `self` and no `that` in the language. The
`self` above is an ordinary lambda parameter, named by whoever wrote the
handler, because a handler is given its own address as its first
argument; call it `me` or `here` and nothing changes. (`this` would not
work there anyway: a handler is refused if it reaches for `this`, since
an actor may not carry the object that spawned it.)

`send` enqueues and returns; `run` delivers until every mailbox is empty.
The checker holds the rules: a handler cannot reach a global `var`, a
mutable global `val`, or `this`, and everything it captures must be
copyable data — because those are exactly the data races a threaded
scheduler cannot allow. And it doesn't: compile this chapter with
`keal build` and every actor runs on its own OS thread, same API, same
output — the interpreters' deterministic round-robin is one legal
schedule, the threads are another.

---

## enum: a closed set of names

```keal
enum Suit { Hearts, Diamonds, Clubs, Spades }

func isRed(s: Suit): Bool {
    return when (s) {
        Suit.Hearts, Suit.Diamonds -> true
        Suit.Clubs, Suit.Spades -> false
    }
}
```

No `else`. Closed means the checker knows every value the type has, so it
can see that those two arms cover it — and the day somebody adds a variant,
every `when` that forgot it says so:

```
error: this `when` over `Suit` does not cover `Jokers`
  = note: add an arm for each, or `else -> ...`
```

That is the whole point of the feature, and it fires in statement position
too. An `else` that can no longer be reached becomes a warning: deleting it
is what puts a later variant back under the guarantee.

`Bool` is closed as well, and so is a nullable enum — the variants, plus
`null`. A variant is an ordinary value: compared, passed, stored in a field,
put in a list, used as a map key, and printed as its bare name.

A variant carries nothing. One that wants fields is a `record`; one that
wants a number is a function with a `when` over the enum, which is an error
the day a variant is added rather than a wrong number.

---

## Collections beyond the three

`List`, `Map` and `String` are built in. Two more are ordinary Keal, in the
prelude — which is the point: a standard library that can only grow by
teaching the compiler a new type has a ceiling.

```keal
val seen: Set<String> = setOf(["a", "b", "a"])
println("${seen.size()} ${seen["a"]} ${seen["z"]}")   // 2 true false
seen["b"] = false                                      // and that removes it

val work: Deque<Int> = dequeOf([1, 2, 3])
work.addFirst(0)
println(work.removeFirst()!!)                          // 0
```

`Set` implements `Index`, which is why `seen[x]` asks and `seen[x] = true`
adds. `Deque` takes from either end without the cost — a queue built on a
list and `removeAt(0)` moves every remaining element and so costs the square
of its length.

Beside them: `distinct`, `zip`, `partition`, `chunked`, `padStart`,
`padEnd`, `lines`.

---

## Your own operators

A class gains an operator by implementing the trait it is wired to — never
by convention, so a class with a `plus` that never said `Add` does not get
`+`.

```keal
class Env(val fallback: Int) : Index {
    func get(name: String): Int { ... }
    proc set(name: String, value: Int) { ... }
}

env["width"] = 80
println(env["width"])
```

`Index` and `Invoke` are the two that carry no signature of their own: what
a class is indexed *by*, and what it gives back, is the class's own
business. `Invoke` makes an object callable — `plus5(10)` — which a lambda
cannot be when it has to carry a `var` field.

---

## What a function may change

A parameter cannot be reassigned — that has always been true here, and it
needs no word. What a parameter *holds* is a separate promise, and Keal keeps
that one too: **the contents belong to whoever passed them**. A function that
intends to change them says so with `var` before the name:

```keal
proc fill(var out: List<Int>, n: Int) {
    for (i in 0..n) { out.add(i * i) }
}

val squares: List<Int> = []
fill(squares, 5)
println(squares)                  // [0, 1, 4, 9, 16]
```

Without the word, every way of changing it is refused — including handing it
to something else that would:

```
error: `xs` is a parameter, so `.add(...)` is not allowed
  = note: the contents of a parameter belong to whoever passed them; write
    `var` before the parameter's name to say this function may change them
```

Reading is always free, and so is building something new: `size`, `sorted`,
`map`, `filter` and the rest answer *about* a value rather than changing it.

---

## `constexpr`: work the compiler does

`constexpr` is a promise about **when** the work happens. The compiler runs
the expression and writes the answer into the program as the literal you
could have typed:

```keal
constexpr val KB = 1024
constexpr val MB = KB * KB              // 1048576, before the program starts

constexpr func squares(n: Int): List<Int> {
    var out: List<Int> = []
    for (i in 1..n) { out.add(i * i) }
    return out
}

constexpr val TABLE: List<Int> = squares(64)   // a literal in the binary
println("${MB} ${TABLE.size} ${TABLE[7]}")
```

```
1048576 63 64
```

A `constexpr func` is one the compile-time evaluator may run. Its body may
use bindings, assignment, `if`, `when`, `while`, `for` and `return` — enough
to build something and ship it as a constant.

Where the promise cannot be kept, the compiler says so rather than quietly
leaving the work for run time: printing, files, `extern`, a lambda, or a call
to a function not declared `constexpr` are all refused by name. Failures are
the program's own, arriving early — `9223372036854775807 + 1` is
`integer overflow` at compile time.

And it always finishes. A `constexpr` gets a step budget and 256 frames, then
it is refused. A compiler that gives a wrong answer is a bug; one that never
answers is not a tool at all.

---

## Macros

A macro is a named piece of syntax, spliced where it is written:

```keal
macro swap(a, b) {
    val held = a
    a = b
    b = held
}

var p = 1
var q = 2
swap!(p, q)
println("${p} ${q}")
```

```
2 1
```

The `!` is not decoration. A macro does three things a function cannot, and
those three are the whole reason it exists.

**Its arguments may be assigned to.** `swap` cannot be a function: what a
parameter holds belongs to its caller, and a function cannot rebind a
caller's name at all.

**Its arguments are expressions, not values.** The body decides whether each
one runs, and how many times:

```keal
macro twice(body) { body  body }
macro discard(unused) { }

var n = 0
twice!(n += 5)         // n is 10
discard!(n += 1)       // n is still 10 — the argument never ran
```

**Control flow passes through it**, because the code ended up where it was
written:

```keal
macro guard(cond, fallback) {
    unless (cond) { return fallback }
}

func describe(n: Int): String {
    guard!(n > 0, "not positive")
    guard!(n < 100, "too big")
    return "ok"
}
```

In statement position the body becomes a block of its own, so the `val held`
in `swap` cannot collide with a `held` you already have — hygiene by scoping
rather than by renaming. In expression position the body must be exactly one
expression, and it takes the call's place:

```keal
macro maxOf(x, y) { if (x > y) { x } else { y } }
println(maxOf!(3, 9))      // 9
```

One limitation, stated rather than left to be found: a parameter stands for
the argument written at the call, but every *other* name in the body resolves
where the macro is expanded, not where it was written.

---

## Depending on someone else's code

A dependency is a git repository at an exact commit, named in `keal.toml`:

```toml
[package]
name = "myproject"
version = "0.1.0"

[dependencies]
geometry = { git = "https://github.com/someone/geometry", tag = "v1.2.0" }
```

`keal fetch` clones each one into `.keal/deps/`, reads its manifest and
fetches what *it* asks for into the same place, and writes `keal.lock`
recording the commit each name resolved to. Then:

```keal
import "dep:geometry/shapes.keal"
```

To find a package whose URL you do not know, there is an index — an ordinary
git repository holding one small file per package, saying where that package
lives and nothing else:

```sh
keal search arithmetic     # find it
keal add geometry          # write it into keal.toml, pinned exactly
keal fetch                 # put it where the import expects it
```

`keal add` with no tag takes the repository's newest version tag and writes
it down as an exact pin — once, here, not again on every build. Nothing
depends on the index existing: a manifest names the package's own repository,
never the index, and a package that is not in the index works just as well by
naming its git URL directly.

---

## Your editor

`keal lsp` is a language server, so one binary gives every editor that
speaks the protocol the same thing: errors as you type, the type of what is
under the cursor, go to definition, find references, rename, an outline, and
completion.

Neovim needs no plugin at all:

```lua
vim.filetype.add({ extension = { keal = "keal" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "keal",
  callback = function(args)
    vim.lsp.start({ name = "keal", cmd = { "keal", "lsp" },
                    root_dir = vim.fs.dirname(args.file) })
  end,
})
```

Helix, Zed and VS Code are about as short —
[`editors/README.md`](editors/README.md) has each of them, plus the syntax
file that JetBrains IDEs read directly.

---

## Where to go next

* [`docs/language.md`](docs/language.md) — the complete reference.
* [`examples/`](examples/) — working programs, including
  [an arithmetic evaluator written in Keal](examples/calculator.keal):
  tokenizer, precedence parser and evaluation in about 120 lines.
* [`tests/programs/`](tests/programs/) — every feature, exercised by
  assertions. These are the most precise description of what the language
  does, because they fail when it stops doing it.
