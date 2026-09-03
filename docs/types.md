# The type rules, written down

The checker is the law; this file is the law stated. Every rule here is
implemented twice (oracle and self-hosted twin, held byte-identical over
the corpus) and exercised by a differential fuzzer
(`tests/fuzz/fuzz.py`) that generates well- and ill-typed programs and
requires (1) the checker never crashes, only accepts or refuses, and
(2) an accepted program means the same thing, byte for byte, to every
engine. Its first run found the three engines disagreeing on one panic
message; the rules below are kept honest by keeping it running.

## The types

`Int` (64-bit, checked), `Float` (IEEE 754 double), `Bool`, `String`
(immutable, UTF-8), `Unit`, `Null`, `Never` (diverges), `Any` (top),
`T?` (nullable), `List<T>`, `Map<K, V>`, `Range`, function types
`(A, B) -> R`, tuples (2–5), classes/records `C<T...>`, and `Param(T)` —
a generic parameter, opaque inside the body that declares it.

## Assignability (`S` flows into `T`)

Reflexive always. Then, in order:

1. `Error` flows anywhere, anywhere flows into `Error` — one diagnostic
   per mistake, no cascades.
2. `Never` flows into everything (a `throw`/`return` fits any hole).
3. Everything flows into `Any`; **`Any` flows into nothing** — the top
   is a sink, not a wildcard. Coming back down takes `is` narrowing.
4. `Null` flows into any nullable. `S? → T?` when `S → T`; `S → T?`
   when `S → T` (values widen into nullability silently, never out).
5. **Containers are invariant** — `List<S> → List<T>` only when the two
   are equal — because they are mutable; the one exception is the empty
   literal, which types as `List<Never>` and fits any list (same for
   `{}` and maps).
6. Functions are **contravariant in parameters, covariant in results**:
   `(A) -> R → (B) -> S` when `B → A` and `R → S`.
7. `C<X...> → C<Y...>` for the same class `C` with pairwise
   *mutually*-assignable arguments (invariance, structurally compared).
8. A type parameter is assignable only to itself (and per rule 3 to
   `Any`). What it can *do* comes from its bounds: `T: Ord` grants
   exactly the trait's methods.

There is no `as` cast and no subtype hierarchy between classes;
inheritance is a non-goal. A trait bound is a capability, not a type.

## Two deliberate wrinkles

* **Integer-literal widening.** An `Int` *literal* (or a literal
  arithmetic expression of them) used where `Float` is expected is
  rewritten to the float in place: `1.5 + 2` is fine, `1.5 + n` is not.
  Value-preserving, literal-only, direction Int→Float only.
* **Operators are traits.** `+ - * / % ** ^/`, unary `-`, `==`, the
  comparisons and `<=>` rewrite to trait-method calls (`Add.plus`,
  `Ord.compareTo`, prelude `compare`, ...); built-ins implement the
  traits like any class would. After the rewrite the ordinary rules
  above apply — there is no separate operator type system.

## Inference (generic calls)

Call-site inference is **fill-and-join with congruence**, not full
unification — a deliberate simplicity, and its limits are stated:

* Each argument is matched against the declared parameter type
  structurally: `Box<List<T>>` against `Box<List<Int>>` binds `T = Int`
  through the nesting (congruence).
* A parameter bound twice **joins**: `f<T>(a: T, b: T)` on `(1, "s")`
  infers `T = Any` — the join lattice below — and then either fits
  (both flow into `Any`) or fails where invariance bites:
  `pair<T>(Box(1), Box("s"))` is refused, because `Box<String>` does
  not flow into `Box<Any>`.
* Lambdas are checked against the parameter types the earlier arguments
  solved, so `deep<T, U>(x: Box<T>, z: (T) -> U)` types the lambda's
  parameter and solves `U` from its body.
* Bounds are checked after solving: `T: Ord` at `T = C` demands the
  class implement the trait, by name.

**The join lattice**: `join(a, a) = a`; `Error` wins; `Never` loses;
`null` adds `?`; nullability distributes (`join(S?, T) = join(S, T)?`);
otherwise the assignable direction wins, and unrelated types join to
`Any`. This is why `if`/`else` branches, ternary branches and `T`-joins
give `Any` rather than an error — using the `Any` is what fails, where
rule 3 stops it.

*The stated debt:* fill-and-join cannot express relations *between*
parameters (`T` must equal `U`'s element, higher-order returns driving
earlier arguments). If those arrive, this section is replaced by real
constraint solving — and this file is where that decision will be
recorded first.

## Narrowing (smart casts)

Facts flow from conditions into the scopes they dominate:

* `x is C` narrows `x` to `C` in the true branch (and with `is C(a, b)`
  binds fields); `not` flips the branches; `x == null` / `x != null`
  narrow nullables to `Null` / `T`.
* **`is` sees only what survives to run time**: the outer shape. Type
  arguments do not (`is List` yes, `is List<Int>` no), type parameters
  do not (`is T`), and neither does a function's signature
  (`is (Int) -> Int`) — each is refused with the reason. A class tests
  as itself, and its fields come back as `Any`.
* `and` narrows its right operand by its left; a condition's facts
  reach `if`/`while` bodies and, negated, the code after a branch that
  always leaves (`return`/`throw`/`break` — the guard idiom, including
  `return if (...)`).
* **Only `val`s narrow.** A `var` could be reassigned between the test
  and the use (by a closure, by a loop), so it never narrows — copy it
  to a `val` first. Fields never narrow for the same reason; read them
  into a local.

## Statement typing

A block's type is its last statement's; `return`/`throw`/`break`/
`continue` type as `Never`, and a block that always diverges is
`Never` — which is how `try { return a } catch (e) { return b }` counts
as returning. `if` without `else` produces no value. `?` selects on
`Bool` with two branches or `Comp` with three, joining the branches.

## Finding an entry in a map

A `Map<K, V>` finds an entry by hashing the key, on all three engines. The
entries themselves are stored in the order they were first set — `keys()`
promises that order, and a removal shifts the tail down rather than swapping
the last entry into the hole — so the index says where to look and never
what the map contains.

The native backend walked the entries until this landed, while both
interpreters had hashed all along: the three agreed on every answer and
differed only in what it cost, which is the kind of difference no test in a
corpus can see.

What that is worth is not one number, and quoting one would be misleading:
the scan's cost is the size of the map, so the ratio is whatever size you
picked. Two hundred thousand lookups, keys built before the clock starts:

| entries | scanning | hashing |
|---|---|---|
| 5 | 0.0023s | 0.0015s |
| 50 | 0.0185s | 0.0022s |
| 500 | 0.1669s | 0.0022s |
| 2000 | 0.6363s | 0.0023s |

The claim is the shape of that second column, not any row of it: **finding an
entry costs the same whatever the map holds.** A small map was never the
problem and is not much improved; a map that grows no longer gets slower.

## A map over a closed key

A `Map<K, V>` whose key type has finitely many values — a `Bool`, a `Comp`,
an enum — stores its entries the way every other map does and finds them
differently. `Bool` has two values, `Comp` has three, an enum has one per
variant, so the ordinal indexes an array of slots and a lookup is a read
rather than a scan.

Nothing a program can observe changes. The entries sit in the order they
were first set, `keys()` promises that order, removal shifts the tail rather
than swapping the last entry into the hole, and re-adding a removed key
appends it at the end. The index follows the entries; the entries do not
follow the index.

**There is one mechanism here, not three container types.** `Map<Bool, V>`
is the map optimised for `true` and `false`; `Map<Comp, V>` is the one
optimised for less, equal and greater; `Map<Level, V>` is the one optimised
for an enum. Naming three of them would ask a reader to choose, and the
choice has one right answer that the compiler already knows.

What it is worth, measured rather than claimed: on a sixteen-variant enum,
four million lookups take a third of the time they did. On a `Bool` or a
`Comp` the scan was already one or two comparisons and the difference is
noise — there the value is the guarantee, not the speed.
