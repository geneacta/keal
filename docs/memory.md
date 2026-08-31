# The memory model

Keal is heading for native code. That makes a question unavoidable which an
interpreter can dodge: **what is a value, in bytes, and who frees it?**

Today every Keal value is a Rust `Value` — a tagged enum whose size and
lifetime Rust decides. Native code cannot borrow that arrangement. This
document is the answer it needs, and `keal layout file.keal` prints it for any
program, so nothing here has to be taken on trust:

```
$ keal layout examples/hello.keal
```

The decision, made once: **reference counting**. Every heap object carries a
count; when the count reaches zero the object is freed. No garbage collector,
no ownership annotations, no borrow checker. That choice is what the rest of
this document elaborates.

---

## 1. Values and references

Two kinds of type, and the difference is visible in the language:

| | Types | Size | Assigning one |
|---|---|---|---|
| **Values** | `Int`, `Float`, `Bool`, `Unit`, `Range` | 8, 8, 1, 0, 16 | copies it |
| **References** | `String`, `List<T>`, `Map<K, V>`, functions, class and record instances | one pointer | shares it, and bumps the count |

There is nothing to box or unbox, and no `int` versus `Integer`. A value type
never touches the heap; a reference type always does.

`Range` is worth noticing: two integers side by side, no allocation. `0..1000`
costs nothing beyond the two numbers.

---

## 2. What an object looks like

Every heap object begins with its reference count, then its contents:

```
Point(val x: Float, val y: Float)

  offset  size  what
       0     8  reference count
       8     8  x
      16     8  y
                                   24 bytes, align 8
```

**Fields keep declaration order.** Reordering them would recover padding —
`Mixed(val flag: Bool, val count: Int, val label: String)` wastes seven bytes
that a sorted layout would not — but a struct whose shape the author cannot
predict is a struct that cannot be handed to C, and cannot be described in a
header file. Predictability and interop win. Where the padding matters, order
the fields yourself; `keal layout` will tell you what it cost.

The count sits **inside** the object rather than beside it. That is what keeps
a reference one pointer wide, which in turn is what lets `String?` be the same
size as `String`.

Each built-in reference type has its own header after the count:

| | after the count |
|---|---|
| `String` | length, then the UTF-8 bytes |
| `List<T>` | length, capacity, a pointer to the elements |
| `Map<K, V>` | length, capacity, the table, the insertion order |
| a function | the code pointer, then the captured values |
| an instance | the fields, in declaration order |

---

## 3. `T?` and spare bit patterns

A nullable is free when the type underneath has a bit pattern to spare, and
costs a tag when it does not:

| | size | why |
|---|---|---|
| `String?`, `List<T>?`, `Point?` | 8 | a pointer is never null while it holds something |
| `Bool?` | 1 | a byte holding 0 or 1 has 254 patterns going spare |
| `Any?` | 16 | `Any` already carries a tag; one more case is free |
| `Int?` | **16** | every bit pattern of an `i64` is a valid `Int` |
| `Float?` | **16** | likewise |
| `Range?` | **24** | likewise, twice |

So making a reference nullable is free, and making an integer nullable doubles
it. That is worth knowing before putting `Int?` in a field of a type you will
have millions of; a sentinel value may be the better trade there, and the
language will not pretend otherwise.

---

## 4. `Any`

A value whose type is not known statically is a tag and a payload, two words
in all — and this is now what the native backend emits, not just what the
layout table priced. The tag is a pointer to the type's static info: its
name (what `typeOf` reads), how to retain and release the payload, how to
render it, how to compare it. `is` is a tag comparison and nothing more;
narrowing casts the payload back to the type the tag names, borrowed —
the tagged variable keeps the reference for the narrowed scope. A null
`Any` is a null tag, which is why `Any?` costs nothing beyond `Any`.

What crosses into an `Any` is exactly what one tag can name: `Int`,
`Float`, `Bool`, `String`, a class at its argument-free or all-`Any`
instantiation, and `List<Any>`. A `List<Int>` does **not** cross — its
elements have their own stride, and an `Any` container holds `Any`
elements — so the backend refuses it by name at the boundary rather than
mis-shaping it. Inside a container, where a slot is one word and an `Any`
is two, the pair lives behind one counted box; that is what lets
`List<Any>` have a stride at all.

Two costs, stated: an `Any` is 16 bytes wherever it is stored, and an
`Any` inside a list is one allocation per element.

---

## 5. Counting

The rules the backend follows:

* A reference is **retained** when it is copied into a new place: a binding, a
  field, a list element, an argument.
* It is **released** when that place goes away: a scope ends, a field is
  overwritten, a list is cleared.
* Releasing the last reference releases each of the object's own fields, then
  frees it.

Value types are never retained or released; there is nothing to count. That is
most of what `is_counted` in `src/layout.rs` decides, and it is why the
arithmetic in a tight loop compiles to arithmetic and nothing else.

The counts are **plain in a program without actors, atomic in one with
them** — one `#define` in the generated C decides, so the cost is paid
exactly where threads exist and nowhere else. Concurrency is **actor-style**
(docs/threads.md): each actor owns its values and messages cross by deep
copy, so a message's *structure* never races. But three kinds of value are
legitimately visible from two threads at once — the addresses (`ActorRef`,
`Outbox`), the strings a copy shares because they are immutable, and
immutable globals — and for those, who-frees-last is only answerable with an
atomic count. Shared-memory threads would have demanded much more: locks
around every mutable container, paid everywhere, to enable races nothing in
the language could check.

### Cycles leak, for now

Reference counting cannot free a cycle. Today one cannot easily be built:
records are immutable, so a record can only refer to values that already
existed. A class with a `var` field is another matter:

```keal
class Node(var next: Node?)
val a = Node(null)
a.next = a          // a cycle; its memory is never returned
```

The interpreter has the same behaviour, since it is built on Rust's `Rc`,
so this is not a regression — but it is a gap, and since `deinit` shipped
the gap grew teeth. **A cycle does not merely leak memory; its `deinit`
never runs.** Both engines and the native backend agree on that today,
silently:

```
built            # the cycle: no deinit, ever
chain built
deinit chain-a   # the acyclic pair dies on schedule
deinit chain-b
done
```

So the question is no longer only "how much memory", it is "which
destructors a program can count on". That deserves an answer, and here
is the reasoning behind the one this project takes.

**The three options, weighed against what Keal has already promised.**

1. **Leave it, documented.** Cheapest, and honest as far as it goes. But
   with `deinit` in the language, "you may not get your destructor" is a
   correctness hole in a feature people are told to rely on.
2. **Weak references.** A `weak Node?` field that does not keep its
   target alive; reading one gives `Node?`, null once the target died.
   The author names the back edge.
3. **A cycle collector**, trial deletion alongside the counts, as CPython
   does. Catches accidental cycles with no annotation at all.

The collector looks like the complete answer, and for CPython it is. For
Keal it collides with four commitments this language has already made:

* **Three engines, one behaviour.** `deinit` is *observable output*. A
  collector in the native runtime that the interpreters cannot run would
  make the same program print different things on different engines —
  the one rule the test suite exists to enforce. And the interpreters
  cannot run one: their values are Rust `Rc`, whose decrements we do not
  own. Collecting there means replacing `Rc` throughout the evaluator
  and the VM — the same ground-up rewrite [`docs/threads.md`](threads.md)
  declined for thread-safety, for the same reason: it taxes every program
  to serve a few.
* **Cycle-free programs must not pay.** Trial deletion works by treating
  every decrement that does *not* reach zero as a cycle candidate: colour
  bits in the header, a candidate buffer, a traversal. That is a cost on
  all programs, including the overwhelming majority that never build a
  cycle. Keal's pattern for optional machinery is a switch the compiler
  throws only when the program needs it (`KEAL_ACTORS`, the `try` mode,
  the `deinit` mode) — and a collector cannot be switched off by a
  program that merely *might* cycle.
* **Destruction is deterministic here.** Values die at statement
  boundaries in reverse declaration order, on every engine. Inside a
  collected cycle there is no last member, so the order of `deinit` calls
  is arbitrary by construction. A collector would carve an exception into
  the one guarantee `deinit` sells.
* **Actors run on real threads now.** Counts go atomic, each actor owns
  its heap. Concurrent trial deletion across those threads is a
  research-grade problem, and the alternative — stopping every actor to
  collect — is exactly the pause a counted language is chosen to avoid.

**The decision: weak references, plus diagnosis for what they miss.**

`weak` is implementable *identically* on all three engines, which is the
argument that settles it. The interpreters get it almost free —
`Rc::downgrade` is the semantics exactly, and `upgrade()` returning
`None` is the null a dead target reads back as. Natively it is the
standard strong/weak header: when the strong count reaches zero the
object runs its `deinit` and releases its fields, and the allocation
survives as a husk until the last weak reference goes, so a weak read is
always a safe read. That second count word is paid only by programs that
write `weak` — the same gating the rest of the runtime already uses.

What weak references do not do is catch the cycle its author did not see.
That is a real gap, and the answer to it is to make cycles *visible*
rather than to collect them:

* the checker can see which class fields are cycle-*capable* — a `var`
  field whose type can reach its own class — and say so where the shape
  is introduced, with `weak` as the suggested fix;
* an opt-in audit at exit can name what was still alive, by type, turning
  "leaked silently" into "leaked, and here is the type to look at".

Neither of those forecloses a collector. If real programs later show that
accidental cycles dominate deliberate ones, trial deletion can still be
added underneath — the header the weak counts introduce is most of what
it would need. That is the order this project prefers: the honest,
cheap, deterministic mechanism first, the automatic one only against
evidence.

*Status: **shipped**. What follows is what it does.*

### `weak`, as built

`weak` is a modifier on a class field, before `val` or `var`:

```keal
class Node(val tag: String) {
    var next: Link? = null
    weak var prev: Link? = null      // the back edge
}
```

* It is a **contextual** keyword, like `record`: `weak` is still an
  ordinary name everywhere else, so no existing program broke.
* The type must be **`T?` where `T` is a class**. A weak reference must
  be able to read back null, and only a counted object can die while
  someone still names it; the checker says so where it is not.
* Reading gives the target while it lives, `null` from the moment its
  last **strong** reference dies. Writing never retains; overwriting
  never releases. Weak edges are invisible to lifetime, so a target's
  `deinit` runs exactly when it would have without them.
* Rendering follows a weak field only while it is alive:
  `Watcher(name="w", watched=Leaf(n=5))` becomes
  `Watcher(name="w", watched=null)` the moment the leaf goes.
* `copy(value)` **refuses** a class with a weak field, by name: an
  address is not a value to duplicate. That refusal is the same
  predicate actors use, so a weak field cannot ride a message either.

How each engine does it:

* **The interpreters** store the slot as a `Weak` handle — reading is
  `upgrade()`, and `None` is `null`. `Rc::downgrade` is the semantics
  exactly, which is why both engines got it for the price of one.
* **The native backend** gates on the program declaring a weak field
  (`KEAL_WEAK` in the emitted C, next to `KEAL_ACTORS`). Under it every
  object carries a second count. When the strong count reaches zero the
  object runs its `deinit` and releases its fields as ever; what changes
  is the last step — it frees itself only if no weak reference remains.
  Otherwise it stays as a **husk**: a header with `rc == 0`, which *is*
  the "dead" test a weak read makes, freed by the last weak release.
  That is why a weak read is always a safe read: the memory it inspects
  cannot have been returned while it still names it.

**The costs, stated:** one extra word per object, and a dead husk
outliving its fields until the last weak reference goes — both paid only
by programs that write `weak`. A program without one emits the C it
always did.

### Diagnosing the cycles `weak` does not catch

A cycle nobody wrote `weak` on still leaks, so the checker cautions about
the one shape where that silently voids something the author asked for:
a class that declares `deinit` and can point a **mutable** field straight
back at its own object.

```
warning: `SelfCycle.back` can point back at its own object, and `SelfCycle` declares `deinit`
  = note: a cycle is never freed, so that `deinit` would never run;
    write `weak` on the back edge to break it
```

The wider rule — *any* field whose type can **reach** the class — was
implemented first and thrown away: it fired thirty-five times on this
compiler's own syntax tree, every one of them a tree that never cycles.
A warning that cries wolf on correct code is worse than no warning. The
narrow rule fires nowhere in this repository except where it is
demonstrated on purpose.

That leaves accidental cycles across several classes undiagnosed by the
checker, and the answer is evidence rather than a better guess. Set
`KEAL_AUDIT` and a program says, on the way out, what it left behind:

```
$ KEAL_AUDIT=1 keal run notes.keal
audit: 2 object(s) outlived the program
  1 Item
  1 Owner
  = note: a class that survives its last reference is in a cycle; `weak` on the back edge breaks it
```

It counts; it does not diagnose. An object that outlives the program is one
whose count never reached zero, and on the interpreters that can only be a
cycle — which is why the report is worth reading even though it names types
rather than objects: the pair of names *is* the shape of the cycle, and
`weak` on one of the two edges is what ends it. Put the word on the back
edge and the same run reports `nothing outlived the program`.

The counters exist only when the variable is set, so a program that does not
ask pays one boolean read per object and prints exactly what it printed
before. The report goes to standard error, so it never joins a program's own
output.

A compiled program answers the same question, asked at build time:

```
$ keal build --audit notes.keal && ./notes
audit: 2 object(s) outlived the program
  1 Item
  1 Owner
```

Same words, same order, same stream — the three engines cannot disagree
about what a program left behind. The switch is a build flag rather than an
environment variable there because a binary cannot grow counters after it is
compiled; without it none of the counting is emitted and no object pays for
it. Under actors the rows go behind the lock the scheduler already owns, so
threads count the same total.

One limit remains, stated rather than left to be discovered: a type that
survives for an ordinary reason — a global that lives to the end of the
program — is reported like any other. The audit is a place to start looking,
not a verdict.

---

## 6. Crossing into C

`keal layout` marks which representations can be handed to C as they are:

* `Int`, `Float`, `Bool`, `Unit` and `Range` cross cleanly. They are `int64_t`,
  `double`, a byte, nothing, and a pair of `int64_t`.
* A `T?` over one of those crosses too: it is a small struct.
* A **record of bare values** crosses **by copy**, as the headerless mirror
  struct `Keal_Name` the generated C defines — same fields, same order, no
  count. Copies carry no ownership, so nothing needs releasing.
* A **`String` crosses only with its ownership written down**: `borrow
  String` hands C the bytes for the duration of a call, `own String` adopts
  a malloc'd buffer C hands back — Keal counts it and frees it. The checker
  refuses a bare `String` at the boundary and says which word to write.
* **Any other pointer to a counted object still does not cross.** It has the
  shape C expects but not the lifetime.

The boundary saying which way ownership moves — rather than passing a
pointer and hoping — was this section's demand from the start; `borrow` and
`own` are that demand kept.

---

## 7. What the native backend does with all this

`src/runtime.c` is this document made real, for the part of the language the C
backend covers: the count at the head of every string, retain and release,
and checked arithmetic so that native code fails where the interpreters fail.

A class becomes exactly the struct described in section 2 — the count, then
the fields in declaration order. Each class gets its own retain and release,
rather than a destructor pointer in the header, because the compiler knows the
static type at every site and the header is meant to stay one word. Releasing
the last reference to an object releases each of the references it held, then
frees it; that cascade is generated from the field types, so it is the layout
table that decides it.

The rule the emitter follows is deliberately blunt: **every counted value is a
named temporary that its block owns, and the block releases it on the way out
by any route.** A binding, an assignment or a returned value takes a reference
of its own. That costs retain/release traffic a smarter pass would elide, and
it is correct without having to reason about any particular path — which
matters more while the rest of the backend is being built. `leaks` reports
nothing outstanding on the test programs, and the suite compares native output
against both interpreters byte for byte.

## 8. Settled, and not

**Settled.** Reference counting. The count inside the object. Fields in
declaration order. Spare bit patterns for nullables where they exist. `Any` as
a tag and a word. Value types never counted.

**Open.** What to do about cycles. Whether counts ever become atomic. How
ownership is spelled at the C boundary. Whether the backend may elide a
retain/release pair it can prove is redundant — it may, and the analysis is
worth having, but correctness comes first.
