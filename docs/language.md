# The Keal language

Keal is a small, statically typed, garbage-free-by-construction scripting
language. It is close in spirit to Kotlin, with a C-family surface syntax:
declared types, mandatory braces, classes with primary constructors, and
null safety enforced by the type checker.

Everything below is implemented and covered by the test suite.

---

## 1. Files, statements and semicolons

A file is a sequence of declarations (`func`, `class`, `import`) and
statements. Statements at the top level run in order; declarations are
visible everywhere in the program, including before the line that declares
them.

```keal
greet()                       // fine: `greet` is declared below

func greet() { println("hi") }
```

### `main`

A program is its top-level statements, and a file needs nothing else to be
one. If it also declares a `main`, that runs last, after the statements
above it:

```keal
println("first")

proc main() {
    println("then main")
}
```

The exit code is `main`'s, when it has one to give:

```keal
func main(): Int {
    return 3                  // the process exits 3
}
```

Either form may take the arguments, which are also reachable from anywhere
with `args()`:

```keal
proc main(argv: List<String>) { println(argv.size) }
```

Those four shapes are all of them. A `main` that is neither is a mistake and
is said so, rather than sitting in the file and never running — which is
what it used to do. Only the entry file's `main` runs: a module that
declares one is a library that can also be run on its own, and importing it
does not start a second program.

The call is appended by the module loader, not by an engine, so what runs is
an ordinary program with one more statement at the end. Nothing below the
loader — neither interpreter, nor the C backend — knows the name `main`.

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
func f() {                     // correct
}

func g()
{                             // error: the newline ended the declaration
}
```

Newlines inside `(...)` and `[...]` never end a statement, so argument lists
and list literals may be spread over several lines freely.

Comments are `// line`, and `/* block */`, which nests.

A file may begin with `#!/usr/bin/env keal`, which is ignored, so a script can
be made executable and run directly.

### Reserved words

Forty-five words are reserved: none of them can be the name of anything.

| | |
|---|---|
| **Declarations** | `func` `proc` `class` `val` `var` `import` `fun` |
| **Visibility** | `public` `private` `package` `internal` `protected` |
| **Control flow** | `if` `unless` `else` `when` `while` `for` `in` `break` `continue` `return` |
| **Errors** | `try` `catch` `throw` |
| **Values** | `true` `false` `less` `equal` `greater` `null` `this` `is` |
| **Connectives** | `not` `and` `or` `xor` `xnor` `nand` `nor` `implies` |
| **Bit operators** | `band` `bor` `bxor` `bnot` `shl` `shr` `ushr` |
| **Held** | `async` `await` `yield` `sealed` `super` `static` `typealias` |

The **held** words name nothing today. They are reserved so that the day one
of those features arrives, no existing program has to be renamed to make room
for it — the same reason `internal` and `protected` are. Writing one is
refused where it appears, and the refusal says what to write instead:

| Held | What to write today |
|---|---|
| `async` `await` | actors: `spawn` a handler, `send` it a message |
| `yield` | a `Sequence`; `seq(xs)` and `iterate(seed, step)` build one |
| `sealed` | an `enum` — a `when` over one needs no `else` |
| `super` | there is no inheritance: compose, or give a trait a default method |
| `static` | a top-level `val` |
| `typealias` | name the type where it is used |

Reserving is not free — it takes a name a program might have wanted — so the
list is short and every word on it has a feature named beside it. Words that
would be plausible names were left out on purpose: `where`, `out`, `init`
and `operator` are all used as ordinary names in this repository, and a
keyword that costs a working program a rename has to earn it.

`fun` is reserved for one reason only: it is what `func` used to be called,
and a file that still says it gets one clear sentence — ``​`fun` is spelled
`func`​`` — instead of a cascade of nonsense from a word read as a name. It
will stop being reserved once nothing is left that spells it the old way.

`internal` and `protected` name no rule today. They are reserved anyway, so
that the day the language grows a visibility between `package` and `public`,
or one that reaches a class's own kind, no existing program has to be renamed
to make room for it. Writing one is refused where it appears rather than
quietly ignored.

Eight more words are **contextual**: they introduce a declaration where one
follows, and stay ordinary names everywhere else.

| Word | Where it means something |
|---|---|
| `record` | before a name: `record Point(...)` |
| `trait` | before a name: `trait Show { ... }` |
| `weak` | before a field: `weak var parent: Node?` |
| `constexpr` | before `val` or `func`: `constexpr val KB = 1024` |
| `macro` | before a name: `macro swap(a, b) { ... }` |
| `enum` | before a name: `enum Suit { Hearts, Spades }` |
| `native` | before a string block |
| `extern` | before `func` or `proc`: `extern func sqrt(...)` |

So `val record = 3` is a perfectly good binding, and always will be. The line
between the two lists is deliberate: a word becomes reserved when reading it
as a name would make a program ambiguous, and not before.

---

## 2. Values and types

| Type | Values | Notes |
|---|---|---|
| `Int` | `0`, `-7`, `1_000_000` | 64-bit signed; overflow is an error, not a wrap |
| `Float` | `1.5`, `-0.25`, `6.02e23` | 64-bit IEEE 754 |
| `Bool` | `true`, `false` | |
| `Comp` | `less`, `equal`, `greater` | what a comparison answers; `Bool`'s three-valued peer |
| `String` | `"text"` | immutable, indexed by character |
| `Unit` | — | what a `proc` produces: nothing. Never written by hand |
| `List<T>` | `[1, 2, 3]` | mutable, ordered |
| `Map<K, V>` | `{"a": 1}` | mutable, insertion-ordered; a key type with finitely many values is indexed, not scanned |
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
| **Values** | `Int`, `Float`, `Bool`, `Comp`, `Unit` | copies it |
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
==  !=  <==>   <  <=  >  >=   is   in   ?:   ..
band bor bxor shl shr ushr
+  -   *  /  %   unary -   unary bnot   . ?. [] ()
```

The bit operators are on a line of their own because their tier is the one
place the table's "tightest binding last" is not the whole story: they bind
tighter than comparison, and against arithmetic they do not bind at all —
mixing the two needs parentheses. The section on them says why.

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
func nonEmpty(s: String?): Bool { s != null implies s.length > 0 }
```

### The bit operators

An `Int` is 64 bits, and seven operators read it as those bits rather than as
the number they spell.

| | Written | Answers |
|---|---|---|
| AND | `a band b` | the bits set in both |
| OR | `a bor b` | the bits set in either |
| XOR | `a bxor b` | the bits set in exactly one |
| NOT | `bnot a` | every bit flipped |
| shift left | `a shl n` | the bits moved up, the top ones discarded |
| shift right | `a shr n` | the bits moved down, the sign carried in |
| shift right, unsigned | `a ushr n` | the bits moved down, zeros carried in |

Words, not sigils. `and`, `or` and `xor` already belong to `Bool` and `^` is
already `xor`, so `&`, `|` and `^` here would each be a second spelling of
something a reader has to look up anyway — and the two meanings of `&` are
exactly what makes C's bit code hard to read. `bnot` is unary and binds where
`not` does.

Both operands are `Int` and so is the result. A `Float` has no bits the
language names, and a `Bool` is one value rather than a row of them — for
those the operator is `and`, `or` or `xor`.

**They mix with nothing.** Two different bit operators side by side is a
syntax error, and so is a bit operator beside an arithmetic one:

```koda
a band b bor c          // error: which applies first?
(a band b) bor c        // fine
a shl 2 + 1             // error
(a shl 2) + 1           // fine — and a different value from a shl (2 + 1)
a band b band c         // fine: the same operator may repeat
```

This is the rule the connectives already follow, for the same reason: where
an order would have to be invented and then remembered, Keal asks. C invented
one — `&` looser than `==`, shifts looser than `+` — and `flags & MASK == 0`
has been quietly meaning `flags & (MASK == 0)` ever since.

What Keal does settle is the case nobody disputes: **bit operators bind
tighter than comparison**, so the test everyone writes needs no parentheses.

```koda
flag band 2 != 0        // (flag band 2) != 0
```

**`shl` truncates.** Bits shifted off the top are gone. This is the only
place in the language where a value is not checked, and it is deliberate:
these operators are defined on the 64 bits an `Int` holds, not on the
magnitude those bits spell, and a `shl` that panicked on overflow would
refuse the one thing it exists for — packing fields into a word.

```koda
1 shl 63                // -9223372036854775808: the top bit is the sign
0xFFFFFFFFFFFFFFFF      // -1, written as the bits it is
```

**A shift count outside `0..63` panics.** It names no shift an `Int` has, so
it is a bug in the program rather than an edge case — and the alternatives
(clamp, count modulo 64, saturate) each turn that bug into a number the
program carries on with. C leaves it undefined, which is the kind of thing
Keal refuses everywhere else.

`shr` carries the sign in, so `-8 shr 1` is `-4` the way `-8 / 2` is; `ushr`
carries zeros in, so `-1 ushr 32` is `4294967295`. Two operators because
there are two answers and neither is the other's default.

Each has a compound form: `band=`, `bor=`, `bxor=`, `shl=`, `shr=`, `ushr=`.

### Hexadecimal and binary literals

`0x` and `0b` write a bit pattern rather than a magnitude, which is the only
reason they exist: a mask is read by its digits, and `16711935` is not
something anyone reads.

```koda
0xFF        // 255
0x00FF_00FF // 16711935, and legible
0b1010      // 10
```

`_` groups digits anywhere in either form, as it does in a decimal literal.
Sixteen hex digits or sixty-four binary ones fit, and the top bit set is a
negative `Int` — the same 64 bits the operators above are defined on.

### `Comp`, the three-valued answer

`Bool` has two values and `Comp` has three: `less`, `equal`, `greater`. They
are the same kind of thing, so they cost the same — one word, nothing to
retain, nothing to free — and they are written the same way, as bare words
rather than members of a type.

```koda
val c = a <=> b          // less, equal or greater
when (c) {               // no `else`: three is all there is
    less    -> "before"
    equal   -> "same"
    greater -> "after"
}
c == less                // what you would have written `c.isLess()`
c != greater             // …and `isAtMost()`
```

**`a <==> b`** asks the order's question about equality: does the comparison
answer `equal`? `a == b` asks `Eq` whether two values are the same; `<==>`
asks `Ord` whether anything separates them. For a type ordered on part of
itself these differ, and both are worth being able to say:

```koda
class Card(val rank: Int, val suit: String) : Ord, Eq {
    func compareTo(other: Card): Comp { this.rank <=> other.rank }
    func equals(other: Card): Bool {
        this.rank == other.rank and this.suit == other.suit
    }
}

Card(7, "hearts") ==   Card(7, "spades")   // false — different cards
Card(7, "hearts") <==> Card(7, "spades")   // true  — the same rank
```

On a primitive the two coincide, because nothing separates two equal `Int`s.

`Comp` carries no methods, for the reason `Bool` carries none. `b.isTrue()`
would be a longer way of writing `b`, and there is no shorter way to say
`c == less` than to say it.

The ternary knows both, and this is the oldest part of the arrangement: a
`Bool` picks between two branches and a `Comp` picks between three, with the
condition evaluated exactly once.

```koda
a <=> b ? "before" : "same" : "after"
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

func double(n: Int): Int { n * 2 }   // no `return` needed
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
func lengthOf(s: String?): Int {
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
func counter(): () -> Int {
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
func describe(v: Any): String {
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
func add(a: Int, b: Int): Int { a + b }      // produces a value, and says which

proc greet(name: String) {                    // produces nothing
    println("hello, ${name}")
}
```

A `func` **must** declare what it returns. A `proc` **cannot**: it returns
nothing, so there is no `Unit` or `void` to write anywhere in the language.
The rule is enforced both ways — `func f(n: Int) { ... }` and
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

Everything below is the same for both, so it says `func` and means either.

```keal
func greet(name: String, greeting: String = "hello"): String {
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
func apply(x: Int, f: (Int) -> Int): Int { f(x) }
apply(5, { it * it })                    // 25

func outer(base: Int): Int {
    func inner(k: Int): Int { base + k }  // captures `base`
    return inner(1) + inner(2)
}
```

A function with a declared return type must produce one on every path, either
by `return` or as its last expression.

### What a function may change

A parameter cannot be reassigned. That has always been true, and it needs no
word: `final` is the default and there is no way to turn it off.

The contents are a separate promise, and Keal keeps that one too. **What a
parameter holds belongs to whoever passed it**, and a function that intends
to change it says so with `var` before the name:

```keal
proc fill(var out: List<Int>, n: Int) {
    for (i in 0..n) { out.add(i * i) }
}

val squares: List<Int> = []
fill(squares, 5)             // and now it holds five
```

Without the word, the checker refuses every way of changing it — a method
that changes its receiver, an assignment that reaches through it, and handing
it on to something else that would:

```keal
proc broken(xs: List<Int>) {
    xs.add(1)                // error: `xs` is a parameter, so `.add(...)` is not allowed
    xs[0] = 3                // error: ... so assigning into it is not allowed
    fill(xs, 2)              // error: ... so passing it as `var out` is not allowed
}
```

That last one is what makes the rest worth anything: a promise that a call
could quietly break is not a promise.

Reading is always free, and so is building something new. `size`, `sorted`,
`map`, `filter`, `keys` — everything that answers *about* a value rather than
changing it — needs no permission. Only six list methods and three map
methods change their receiver, and those are the only ones the word is about.

**The boundary, stated rather than left to be found.** `var` describes what
*this function and the calls it makes* will do. It is not a claim about the
value forever: a function may still store what it was given somewhere that
outlives the call, and whoever holds it afterwards is bound by nothing. Doing
better than that means tracking a borrow through the heap, which is a
different and much larger language than this one. What is here is the promise
a signature can keep, and it keeps it.

---

## 8. Classes

```keal
class Point(val x: Float, val y: Float) {
    val magnitude: Float = sqrt(x * x + y * y)   // sees constructor parameters
    var label: String = "?"

    func plus(other: Point): Point {
        return Point(this.x + other.x, this.y + other.y)
    }

    func toString(): String { "${this.label}(${this.x}, ${this.y})" }
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
* Defining `func toString(): String` changes how `println` and interpolation
  render the instance; otherwise they print `Point(x=3.0, y=4.0)`.
* Instances compare by **identity**. Structural comparison would loop forever
  on a cyclic object graph, so `==` on two separately built instances is
  `false`.
* There is no inheritance and no static members. Traits are how a type
  promises behaviour; a value that belongs to no instance is a top-level
  `val`.

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
    func compareTo(other: Version): Comp {
        if (this.major != other.major) { return this.major <=> other.major }
        return this.minor <=> other.minor
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
func area(shape: Any): Float {
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
func divmod(a: Int, b: Int): (Int, Int) {
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

## 9½. enum

An enum is a closed set of names:

```keal
enum Suit { Hearts, Diamonds, Clubs, Spades }

val trump: Suit = Suit.Spades
println(trump)              // Spades
println(typeOf(trump))      // Suit
```

**Closed is the whole of it.** The checker knows every value the type has, so
a `when` over one needs no `else`:

```keal
func isRed(s: Suit): Bool {
    return when (s) {
        Suit.Hearts, Suit.Diamonds -> true
        Suit.Clubs, Suit.Spades -> false
    }
}
```

and the day somebody adds a variant, every `when` that forgot it says so:

```
error: this `when` over `Suit` does not cover `Jokers`
  = note: add an arm for each, or `else -> ...`
```

That is the feature. It fires in statement position too — a `when` that
dispatches on a variant and forgets one is exactly the case worth catching,
and it usually produces no value. An `else` that can no longer be reached is
a **warning**, not an error: deleting it is what puts a later variant back
under the guarantee.

`Bool` is closed as well, and always was. So is a nullable enum — the
variants, plus `null`:

```keal
func label(s: Suit?): String {
    return when (s) {
        null -> "none"
        Suit.Hearts, Suit.Diamonds -> "red"
        Suit.Clubs, Suit.Spades -> "black"
    }
}
```

A guarded arm covers nothing, exactly as it has never counted as an `else`.

**A variant is an ordinary value.** It is bound, compared with `==`, passed,
returned, stored in a record field, put in a list, and used as a map key.
It renders as its bare name. Two enums may share a variant name, because a
variant lives inside its enum and is always written `Suit.Hearts`.

Natively an enum is one word — an ordinal. Nothing to retain, nothing to
free: it is the cheapest thing in the language to send to an actor.

### What an enum refuses

| | |
|---|---|
| `enum Shape { Circle(r: Float) }` | a variant that carries something is a `record` |
| `enum Http { Ok = 200 }` | write a function with a `when`, so adding a variant is an error rather than a wrong number |
| `enum Empty { }` | a type with no values cannot be built; `Nothing` already means that |
| `Hearts` bare, or `.hearts` | one spelling, `Suit.Hearts`, so two enums may share a name |
| `Suit.Hearts < Suit.Spades` | declaration order is a spelling decision, not a semantic one |
| a variant named `values` | `values()` is the list of an enum's variants |

**No payloads, deliberately, for now.** A variant carrying data would need a
case-to-enum assignability edge — this language's first subtyping relation,
in a language whose §20 lists inheritance as a non-goal — threaded through
assignability, joining *and* unification. What a program has meanwhile is
`throw`/`catch` for failure and `T?` for absence, which are the two things
payload enums are mostly used for, plus the record-with-a-tag idiom this
compiler itself uses — whose tag `enum` upgrades from a `String` to a checked
type, and whose `when` it makes exhaustive.

---

## 10. Generics

Functions and classes may take type parameters. They are written after the
name, as in `func name<T>(...)` and `class Box<T>`:

```koda
func firstOr<T>(xs: List<T>, fallback: T): T {
    for (x in xs) { return x }
    return fallback
}

func mapAll<T, R>(xs: List<T>, f: (T) -> R): List<R> {
    val out: List<R> = []
    for (x in xs) { out.add(f(x)) }
    return out
}

class Box<T>(val value: T) {
    func get(): T { this.value }
    func then<R>(f: (T) -> R): Box<R> { Box(f(this.value)) }
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
    func show(): String
    func shout(): String { this.show().toUpper() }   // a default body
}

trait Ordered {                                    // your own trait, not `Ord`
    func compareTo(other: Self): Int
}

class Version(val major: Int, val minor: Int) : Show, Ordered {
    func show(): String { "v${this.major}.${this.minor}" }
    func compareTo(other: Version): Int {
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
func describe<T: Show>(value: T): String { value.show() }

func loudest<T: Show + Ordered>(a: T, b: T): String {
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
| `a + b` | `Add` | `func plus(other: Self): Self` |
| `a - b` | `Sub` | `func minus(other: Self): Self` |
| `a * b` | `Mul` | `func times(other: Self): Self` |
| `a / b` | `Div` | `func div(other: Self): Self` |
| `a % b` | `Rem` | `func rem(other: Self): Self` |
| `-a` | `Neg` | `func negate(): Self` |
| `a == b`, `a != b` | `Eq` | `func equals(other: Self): Bool` |
| `a < b`, `<=`, `>`, `>=` | `Ord` | `func compareTo(other: Self): Comp` |
| `a[i]` | `Index` | `func get(key): Value` |
| `a[i] = v` | `Index` | `proc set(key, value)` |
| `a(x, y)` | `Invoke` | `func invoke(...): Result` |

`Index` and `Invoke` are the two that carry no signature of their own, and
that is deliberate: what a class is indexed *by*, and what it gives back,
differ from class to class. So the trait says a class is indexable and the
class's own `get` says with what — the key may be a `String`, the value
anything. A class that declares `Index` and has no `get` is refused where it
says so, not at a use site three files away.

```keal
class Env(val fallback: Int) : Index {
    func get(name: String): Int { ... }
    proc set(name: String, value: Int) { ... }
}

env["width"] = 80
println(env["width"])
```

Nothing here works by convention: a class with a `get` that never said
`Index` is not indexable. An operator is a promise a class makes, not a
shape the checker guesses. And `a[i] += x` is refused — write it out, so the
`get` and the `set` are both visible.

```koda
class Vec2(val x: Float, val y: Float) : Add, Neg, Eq {
    func plus(other: Vec2): Vec2 { Vec2(this.x + other.x, this.y + other.y) }
    func negate(): Vec2 { Vec2(-this.x, -this.y) }
    func equals(other: Vec2): Bool { this.x == other.x and this.y == other.y }
}

Vec2(1.0, 2.0) + Vec2(3.0, 4.0)     // Vec2(4.0, 6.0)
```

`a + b` on a class is *rewritten* into `a.plus(b)`, and `a < b` into
`a.compareTo(b) == less` — `<=` into `a.compareTo(b) != greater`, and so on.
There is no sign to inspect and no zero to compare against. The methods stay
ordinary methods: you can call `a.plus(b)` yourself.

**Equality is the exception.** `==` already works on every type by comparing
identity, so a class without `Eq` is not an error — it simply keeps comparing
identity, and two separately built instances are never equal. Implementing
`Eq` is how a class opts into structural equality.

**The built-in types implement these traits too.** `Int` and `Float` provide
all of them, `String` provides `Add`, `Eq` and `Ord`, and `Bool` provides
`Eq`. So a bound is satisfied by a built-in as readily as by your own type:

```koda
func total<T: Add>(xs: List<T>, zero: T): T {
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
func rounded(x: Float): Int { ... }        // this file's business
package func parse(src: String): Ast { ... }  // the files beside it
public class Ast(val root: Node) { ... }     // anyone who imports it
```

| Written | Who may name it |
|---|---|
| nothing, or `private` | the file that declares it |
| `package` | every file in the same directory |
| `public` | every file that imports it |

That table is the whole rule, and it applies to **every** top-level
declaration in the same way: `func`, `proc`, `class`, `record`, `trait`,
`enum`, `macro`, `extern`, and a top-level `val` or `var`. Reading a name and
assigning to one are the same question — a file that may not read a `var` may
not write it either.

### Inside a class or a record

A member may carry its own modifier, and what an unwritten one means is the
one place the two differ:

| | A member that says nothing |
|---|---|
| `class` | **private**, like a top-level declaration |
| `record` | **as visible as the record itself** |

A record *is* its fields: a record whose data cannot be read is not the data
case, so `public record Point(val x: Int)` exposes `x`. A class keeps its own
counsel, so `public class Counter(val n: Int)` exposes the type and nothing
else — `n` has to say `public` to be read from another file.

```keal
public class K(public val open: Int, val shut: Int) {
    public func visible(): Int { return 1 }
    func hidden(): Int { return 2 }        // private, though the class is public
}
public record R(val open: Int)             // `open` is public, because R is
```

Three details that follow from the rule rather than adding to it:

* **A method that answers a trait is always reachable.** `a + b` is
  `a.plus(b)`, so refusing it by its own modifier would make an operator
  depend on where it is written. A class that says it implements a trait has
  promised the trait's methods.
* **A declaration always reaches its own file**, whatever it says. `private`
  is about other files, not about the line below it.
* **A trait is a bound, not a type.** `func f<A: Ord>(a: A)` is how a trait is
  written; `func f(a: Ord)` is not a thing. So a trait's visibility governs
  who may bound a type parameter by it, and its methods are reached through
  whatever implements it.

A **package is a directory**. Nothing declares it and nothing names it: the
files that sit together are the ones that can see each other's `package`
declarations, which is what lets a group of files collaborate without
promising anything to the outside.

### Naming what you import

Two files may declare `parse`. The file that imports them says which it
means:

```keal
import "./lexer.keal"                 // its names, bare
import "./config.keal" as config      // its names, through `config`
```

An aliased import contributes nothing to the bare set: `parse` is the
lexer's, `config.parse` is the other, and `config.Node` names its type in a
type position too. Writing a name that two visible modules declare is an
error **where the name is written**, naming both files — never at the
import, so two modules that happen to share a name cannot break a program
that never mentions it.

### Depending on somebody else's code

An import names a file. A dependency's file is named through `dep:`:

```keal
import "dep:geometry/shapes.keal"
```

which reads `.keal/deps/geometry/shapes.keal` beside the nearest
`keal.toml`. That manifest names the project and lists what it depends on,
each as a git repository at an exact tag or commit:

```toml
[package]
name = "myproject"
version = "0.1.0"

[dependencies]
geometry = { git = "https://github.com/someone/geometry", tag = "v1.2.0" }
```

`keal fetch` clones each one and checks out what was named. Nothing else
in the compiler touches the network: a `dep:` import reads what is on disk,
so a project that commits `.keal/deps/` builds without git.

There is no `package` declaration and no version resolution — a manifest
names a commit, not a range. There *is* an index: `keal search` finds a
package whose URL you do not know, `keal add` writes it into `keal.toml`
pinned to one exact tag, and the index is an ordinary git repository rather
than a service anybody has to keep running.
[Packages and namespaces](packages.md) argues the order and the difference.

The modifier goes on a top-level `func`, `proc`, `class`, `record`, `trait`,
`extern func`, `val` or `var` — and on a class's own members:

```keal
public class Counter(public var n: Int) {
    var steps: Int = 0                    // the class's own business
    public proc bump() { this.n += 1; this.steps += 1 }
    proc audit() { ... }                  // likewise
}
```

A **class keeps its own counsel**: a member that says nothing is private,
like a top-level declaration. A **record is its fields**, so a field that
says nothing is as visible as the record itself — a record whose data cannot
be read is not the data case. Writing a modifier on a record's field is
still obeyed:

```keal
public record Marker(val x: Int, private val salt: Int)
```

A method that answers a trait the class implements is always reachable,
whatever it says: `a + b` is `a.plus(b)`, and an operator must not depend on
which file it is written in.

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
| `time(): Float` | seconds since the Unix epoch, UTC |
| `distinct(xs)` | the values of `xs`, duplicates dropped, first kept |
| `zip(a, b): List<Tuple2<A, B>>` | pairs off two lists, stopping at the shorter |
| `partition(xs, keep): Tuple2<List<T>, List<T>>` | what passes the test and what does not, in one pass |
| `chunked(xs, n): List<List<T>>` | `xs` in runs of `n`, the last one short if it has to be |
| `padStart(s, width, pad)` `padEnd(...)` | pad to a width; never truncates |
| `lines(s): List<String>` | the lines, without their newlines |
| `setOf(xs)` `dequeOf(xs)` | the two collections below, from a list |

### Regular expressions

Not built in, and in `lib/regex.keal` rather than the prelude:

```keal
import "lib/regex.keal"

val r = Regex("([0-9]+)-([0-9]+)")
val m = r.find("order 12-345 shipped")
if (m != null) { println("${m.text} ${m.groups[0]} ${m.groups[1]}") }
```

| | |
|---|---|
| `Regex(pattern)` | refuses a pattern it cannot read, with a message |
| `matches(text)` | the whole text, or nothing |
| `find(text): Match?` `findFrom(text, at)` | the first match |
| `findAll(text): List<Match>` | every non-overlapping match |
| `replace(text, with)` | `$0` the whole match, `$1` the first group, `$$` a dollar |
| `split(text): List<String>` | the pieces between the matches |

`Match` carries `text`, `start`, `end` and `groups: List<String?>` — a group
that did not take part is `null`, which is not the same answer as one that
matched the empty string.

The syntax is the common core: `.`, `[abc]` with ranges and negation, `\d`
`\w` `\s` and their negations, `^` `$`, `*` `+` `?` and `{n,m}` in greedy
and lazy forms, `(` `)` and `(?:` `)`, and `|`. No backreferences, no
lookaround, no named groups — each of those changes what the matcher is
rather than what it knows.

**It is written in Keal.** Nothing in a matcher is a system call, and a Keal
string is indexed by character rather than byte, so `.` matches one character
however many bytes it takes and `日+` matches `日日`. Written this way it also
needs no second implementation: all three engines run the same source, so
they cannot disagree about it the way they could about anything written
twice. It is a library rather than the prelude because the C backend emits
the whole prelude into every generated program, and a program that wants no
regular expressions should pay for none.

Two things to know before relying on it. `\d`, `\w` and `\s` are **ASCII**,
as they are in nearly every engine: `\w+` against `"héllo"` matches `"h"` and
stops. Everything else is fully Unicode, so this is the one place a pattern
means less than a Keal string does. And matching is backtracking, which a
pattern as short as `(a+)+b` can drive to exponential time — so an attempt
that runs past a million steps **throws** rather than hanging, because a
program that hangs cannot be told from one that is working.

### Running another program

| | |
|---|---|
| `runCommand(argv: List<String>): List<String>?` | `[exit code, standard output, standard error]`, or `null` if it could not be started |

No shell is involved. The list *is* the argument vector, so a path with a
space in it stays one argument and nothing is ever re-parsed — which is the
difference between running a program and handing a string to `sh` and hoping.

```keal
val r = runCommand(["git", "rev-parse", "HEAD"]) ?: []
if (r.size == 3 and r[0] == "0") { println(r[1]) }
```

`null` means the program could not be started; a program that ran and
failed comes back with its exit code. Confusing the two is how a script
retries the wrong thing.

It compiles natively too, and the part of that worth knowing is why it took
a second pass. Draining one stream to the end before the other **deadlocks**
the moment the child fills the other stream's pipe buffer — 65536 bytes on
Unix, and 4096 on Windows, which is not a stress case but any command that
writes a result and a warning. So both are drained at once: `poll` on Unix, a
reader thread per stream on Windows.

On Windows the wide entry points are used rather than the ANSI ones, and that
was measured rather than assumed: the ANSI form appears to round-trip UTF-8
perfectly into the child's own `argv`, while the command line Windows
actually built is mojibake — so a child that reads the wide command line, or
opens an argument as a path, gets a name that is not the one you passed. A
Keal string is UTF-8, so it is converted at the boundary and quoted by
`CommandLineToArgvW`'s own rules.

**How a name is looked up.** A first argument with no directory in it is a
name to be resolved; one with a directory is a location to be used. `sh`
searches, `./bin/sh` does not, and a name containing a directory that holds
nothing is not rescued by the search path. That much is the same on every
platform and on all three engines.

What is *not* the same is Windows, and it is the platform's convention
showing through rather than a decision of this language:

* **The current directory is searched first, natively.** Compiled code runs
  a `git.exe` sitting in the working directory in preference to the
  installed one; the interpreters go through Rust's resolution, which
  deliberately excludes it, and answer `null`. A bare name can therefore
  mean two different programs depending on the engine. Until that is
  aligned, pass a path rather than a name for anything that must not be
  substitutable by a file someone else can drop next to yours. On Unix the
  current directory is consulted only when `PATH` asks for it — a `.` entry
  or an empty one — and all three engines then agree, including on where in
  the order it falls.
* **Only `.exe` is appended, natively.** `PATHEXT` is not consulted, so a
  bare name never reaches a `.bat` or a `.cmd`. Name the file.
* **Use backslashes when the program is a `.bat`.** A batch file is not an
  executable image: Windows runs it through `cmd.exe`, which re-reads the
  line, and a forward slash there begins a switch. `.\tool.bat` works where
  `./tool.bat` does not, and `C:/Windows/System32/cmd.exe` loses the child's
  exit code where `C:\Windows\System32\cmd.exe` keeps it. Forward slashes
  are fine everywhere else in this language, `isFile` and `listDir`
  included, which is exactly why this one is worth writing down.

### Dates and times

`time(): Float` is seconds since the Unix epoch, and a calendar is written
over it in the prelude — no primitive of its own, so a program that wants a
different calendar can read this one and write another.

| | |
|---|---|
| `utcAt(seconds: Int): DateTime` `utcNow()` | a moment on the UTC clock |
| `localAt(seconds: Int): DateTime` `localNow()` | the same moment on this machine's clock |
| `localOffset(at: Int): Int` | seconds east of UTC **at that instant** |
| `daysFromCivil(year, month, day): Int` | the inverse of the calendar, exactly |
| `monthName(m)` `weekdayName(w)` | January is 1, Sunday is 0 |
| `isLeapYear(year): Bool` | |

`DateTime` is a record — so two moments built the same way are equal — with
`year`, `month`, `day`, `hour`, `minute`, `second`, `weekday` (0 is Sunday)
and `offset` (seconds east of UTC, 0 for UTC), and:

| | |
|---|---|
| `iso()` | `2026-09-01T07:29:27Z`, or `2026-09-01T09:29:27+02:00` |
| `zone()` | `Z`, `+02:00`, `-05:30`, `+12:45` |
| `date()` `clock()` | `2026-09-01` and `07:29:27` |
| `epochSeconds()` | back to where it came from, offset removed |
| `inUtc()` `inLocalTime()` | the same moment, read on the other clock |

```keal
println(utcNow().iso())                  // 2026-09-01T07:29:27Z
println(localNow().iso())                // 2026-09-01T09:29:27+02:00
println(localNow().inUtc() == utcNow())  // true — one moment, two clocks
```

A moment **carries** its offset rather than assuming one, so its numbers
always say which clock they are on. `localOffset` takes the instant, not
"now": most of the world is on two different offsets across a year, and a
calendar that asks what the offset is today gets the other half of the year
wrong by an hour. Zones offset by 30 and 45 minutes are ordinary here —
Kolkata is `+05:30`, Chatham is `+12:45`.

The offset comes from the C library, and neither the interpreters nor the
generated C ever reads a field of `struct tm` — the C standard fixes which
members it has but not their order, and the two interpreters would have to
declare that layout in Rust to read it. The pointer goes straight into
`strftime`, which prints the offset, and only that is read. On any failure
the answer is UTC, which is at least true and is labelled honestly.

### Files and directories

Reading and writing a whole file are `readFile(path): String?` — `null` when
it cannot be read — and `writeFile(path, content): Bool`.

Four more primitives reach the file system, and no more. A built-in name is
reserved for good, so only a system call earns one:

| | |
|---|---|
| `listDir(path): List<String>?` | the entry names, **sorted**; `null` if `path` is not a directory |
| `pathKind(path): Int` | `0` nothing, `1` a file, `2` a directory |
| `makeDir(path): Bool` | the directory and every parent it needs; true if it is there afterwards, so making one twice is not a failure |
| `removePath(path): Bool` | one file, or one *empty* directory — never a tree |

The names a program actually writes are ordinary functions in the prelude,
which means a program that wants its own may simply declare one:

| | |
|---|---|
| `exists(path): Bool` | anything at all is there |
| `isFile(path)` `isDir(path)` | which of the two it is |
| `walkDir(path): List<String>` | every file underneath, depth first, each readable |

`listDir` sorts because a directory hands its entries out in whatever order
its file system pleases, and the three engines have to print one order — a
program that lists a directory has to say the same thing on every machine it
runs on.

A path is UTF-8, and stays UTF-8 all the way to the operating system. On
Windows that means the wide entry points rather than the ANSI ones, which
read a name as the active code page: a program creating `日本` through those
would put `æ—¥æœ¬` on disk and then list `日本` back — self-consistent, and a
name no other tool on the machine can open. Worse, a file some other program
created could not be seen at all. Nothing written in Keal could detect that,
since a program that makes its own tree agrees with itself; the test that
catches it makes the directory with `runCommand` and then asks `listDir` what
it sees.

`removePath` stops at one entry on purpose. A recursive delete behind a
one-word name is how a program loses what it did not mean to; a program that
wants a tree gone can walk it and say so.

```keal
for (f in walkDir("src")) {
    if (f.endsWith(".keal")) {
        println("${f}: ${lines(readFile(f) ?: "").size} lines")
    }
}
```

### `Set<T>` and `Deque<T>`

Neither is built into the compiler. Both are ordinary Keal, in the prelude,
and that is the point: a standard library that can only grow by teaching the
compiler a new type is a standard library with a ceiling. A program can
write its own the same way.

`Set<T>` is membership without order or duplicates, backed by a map — so
anything a map can key, a set can hold. It implements `Index`, so `s[x]`
asks and `s[x] = true` / `s[x] = false` add and remove.

| | |
|---|---|
| `add(v)` `remove(v)` | |
| `s[v]: Bool` | membership, through `Index` |
| `size()` `isEmpty()` | |
| `toList()` | the values, in the order first added |

`Deque<T>` is a queue you can take from either end. A list can do this
already — `add` at the back, `removeAt(0)` at the front — but `removeAt(0)`
moves every remaining element, so a queue built that way costs the square of
its length. This one keeps a head index and compacts only when the wasted
front is most of the buffer.

| | |
|---|---|
| `addFirst(v)` `addLast(v)` | |
| `removeFirst(): T?` `removeLast(): T?` | `null` when empty — a question, not a failure |
| `first(): T?` `size()` `isEmpty()` `toList()` | |

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

### throw and catch

`throw` raises any value; `try` runs a block and `catch` takes what it
raised:

```keal
record Refused(val why: String, val at: Int)

try {
    if (n > 10) { throw Refused("too big", n) }
    if (n < 0) { throw n }
    use(n)
} catch (e: Refused) {
    println("refused ${e.why} at ${e.at}")
} catch (e: Int) {
    println("an Int: ${e}")
} catch (e) {
    println(e)
}
```

Clauses are tried in order. **A clause that names a type** takes only what
that type can hold, and binds the value whole. **A clause that names none**
takes anything and binds the *message* — the value as a program would print
it — which is why it always has something to say and why it must come last;
a clause behind it is refused as unreachable.

When no clause matches, the value goes on unwinding to the `try` outside,
unchanged. And every built-in failure raises its own message, so
`catch (e: String)` catches an overflow as readily as a `throw "..."`.

`return`, `break` and `continue` are jumps, not failures, and pass through a
`try` untouched. A `try` whose body and whose catch-all both return counts as
returning, like an `if`/`else` that does — a typed clause alone does not,
because it may not run.

One thing cannot be thrown: a function. A signature has no run-time identity,
so no `catch` could name one.

All three engines catch by type. Natively the thrown value rides the unwind
as an `Any` — tag and payload, the same pair `is` tests — so a clause is a
tag compare, and the rules above hold on the compiled program exactly as
they hold on the interpreters. The one consequence: a value the native
backend cannot put in an `Any` cannot be thrown in a compiled program, and
`keal build` says so by name rather than compiling something else.

---

## 15½. constexpr

`constexpr` is a promise about **when** the work happens. The compiler runs
the expression, and writes the answer back into the program as the literal
you could have typed:

```keal
constexpr val KB = 1024
constexpr val MB = KB * KB          // 1048576, before the program starts
constexpr val BANNER = ("the " + NAME).toUpper()
```

A `constexpr func` is a function such a binding may call. Its body may use
bindings, assignment, `if`, `when`, `while`, `for`, `break`, `continue` and
`return` — enough to build something:

```keal
constexpr func squares(n: Int): List<Int> {
    var out: List<Int> = []
    for (i in 1..n) { out.add(i * i) }
    return out
}

constexpr val TABLE: List<Int> = squares(64)   // a literal in the binary
```

The value must be one a literal can spell: `Int`, `Float`, `Bool`,
`String`, and lists and maps of those. Adding to a container works, and
only through a name the `constexpr` bound itself — that is where the folder
can see what it is changing.

**What it refuses, and why it refuses rather than falls back.** Anything
that touches the world (printing, files, `extern`, `native`, actors), an
object, a lambda, `null`, `this`, or a call to a function not declared
`constexpr`. A `constexpr` that quietly ran at run time instead would make
the word worth nothing, so where the promise cannot be kept the compiler
says so by name:

```
error: `constexpr` cannot evaluate a lambda
  = note: a `constexpr` runs at compile time, so it is held to arithmetic,
    strings, lists, maps and calls to other `constexpr func`s
```

Failures are the program's own failures, arriving early. `9223372036854775807 + 1`
is `integer overflow` at compile time; `[1, 2][5]` is `index 5 is out of
bounds for a list of 2 element(s)`. Nothing about the arithmetic changes —
only when you find out.

**It always finishes.** A `constexpr` gets a step budget and 256 frames.
Past either, it is refused:

```
error: this `constexpr` did not finish
  = note: it ran past the compile-time step budget; a loop that does not
    end at compile time would be a compiler that does not end
```

That limit is the point. A compiler that gives a wrong answer is a bug; a
compiler that never answers is not a tool at all.

`constexpr` is contextual, so `val constexpr = 7` is still a perfectly good
binding. It goes before `val` and `func` only: a `var` can be assigned to and
a `proc` returns nothing, so neither has one value to compute.

---

## 15¾. macro

A macro is a named piece of syntax, spliced where it is written:

```keal
macro swap(a, b) {
    val held = a
    a = b
    b = held
}

var p = 1
var q = 2
swap!(p, q)          // p is 2, q is 1
```

The `!` at the call is not decoration. A macro can do three things a
function cannot, and those three are the whole reason it exists:

* **Its arguments may be assigned to.** `swap` cannot be a function here:
  what a parameter holds belongs to whoever passed it, and a function cannot
  rebind a caller's name at all.
* **Its arguments are expressions, not values.** The body decides whether
  each one runs, and how many times:

  ```keal
  macro twice(body) { body  body }
  macro discard(unused) { }

  var n = 0
  twice!(n += 5)     // n is 10
  discard!(n += 1)   // n is still 10 — the argument never ran
  ```

* **Control flow passes through it**, because the code ended up where it was
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

A reader has to be able to tell those apart from a call, and the `!` is how.

**Where it goes.** In statement position the body becomes a block of its
own, so the `val` in `swap` cannot collide with a `held` the caller already
has. That is hygiene by scoping rather than by renaming. In expression
position the body must be exactly one expression, which then takes the
call's place:

```keal
macro maxOf(x, y) { if (x > y) { x } else { y } }
println(maxOf!(3, 9))      // 9
```

A macro whose body is more than one statement, used where a value is
wanted, is refused by name.

**What resolves where.** A parameter stands for the argument written at the
call. Every *other* name in the body resolves **where the macro is
expanded**, not where it was written — a macro that calls `helper()` reaches
the caller's `helper`. That is a real limitation, and it is stated here
rather than left to be discovered.

**It always finishes.** A macro that expands to itself would be a compiler
that does not end, so expansion gets 64 levels and then says so. A macro
body may not declare a function or a class either: a declaration spliced
twice is two declarations of one name.

**What a macro is not.** It does not take a type, produce a declaration, or
run a program at compile time to write code. Those want an AST a program can
hold as a value, which is a much larger language — and this one has not
earned it.

`macro` is contextual, so `val macro = 3` is still a perfectly good binding.

---

## 16. How a program runs

Keal compiles to bytecode and runs it on a virtual machine. That is an
implementation detail with one visible consequence: `keal --ast file.keal`
runs the same program on the tree-walking evaluator instead, and the two are
required to agree on every byte they print. If they ever do not, that is a
bug — please report it with the program that shows it.

## 17. Calling C and C++

A `native` block passes text verbatim into the C that `keal build` generates:
headers, inline helper functions, declarations. `extern func` then binds a
symbol with a signature the checker enforces at every call:

```keal
native """
#include <math.h>
static int64_t triple(int64_t n) { return n * 3; }
"""

extern func sin(x: Float): Float
extern func triple(n: Int): Int
extern func pow(base: Float, exponent: Float): Float = "pow"
extern proc srand(seed: Int)
```

The `= "symbol"` names the C symbol when it differs from the Keal name.

`extern proc` is the boundary's `void`. The distinction the language makes
between `func` and `proc` is the one C makes between a return type and
`void`, so the declaration keeps it: a C function that returns nothing is
declared as returning nothing, rather than claiming an `Int` its caller then
has to ignore.

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

`keal lsp` is a language server. It speaks the Language Server Protocol over
stdin and stdout, so one binary serves every editor that speaks it —
[`editors/README.md`](../editors/README.md) has the four lines each of
Neovim, Helix, Zed and VS Code need.

| | |
|---|---|
| Diagnostics as you type | errors and warnings, note included |
| Hover | the type of the thing under the cursor |
| Go to definition | for a name, a field, a method, an enum |
| Find references, rename | every use, and all of them at once |
| Outline | the declarations in a file |
| Completion | the names in scope, and the keywords |

It is not a second implementation of the language. It loads and checks the
file exactly as `keal check` does, reading the editor's unsaved buffer
instead of the disk — so it cannot drift from the compiler, and a wrong
answer here would be a wrong answer there.

**What it does not do yet**, stated rather than left to be found: it
re-checks the whole file on every keystroke (fine for a file, slow for a
very large program), member completion after a `.` offers the names in scope
rather than that value's members, and there are no code actions, no
formatting, and no inlay hints.

[`editors/vscode`](../editors/vscode) also holds the syntax file:
highlighting, bracket and indent behaviour, and snippets. The grammar is a
standard TextMate file that Sublime Text and JetBrains IDEs read directly.

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

Class inheritance (a non-goal) · associated types on traits · generic
traits · enum variants that carry data · `?.` on a built-in receiver in the
C backend (refused by name; the interpreters answer `null`) · `List<Int?>`
and the other lists of nullable value types · a network stack
(HTTPS needs TLS, which belongs behind the interop boundary rather than
hand-written in the runtime).

Shipped since this list was first written, and no longer on it: `throw` /
`try` / `catch` on all three engines — typed clauses included, natively —
destructuring a record in a `when` or a binding, the native backend
through C11 (with C, C++, Rust, Go, Java and Kotlin interop), actors on
real OS threads, `deinit`, `weak`, `Any` natively, visibility with
`package` and `public`, a namespace that lets two modules declare the same
name, dependencies with transitivity and a lockfile, `constexpr`, and
macros. What the C
backend still refuses, it refuses **by name** — `keal build` never
mis-compiles.
