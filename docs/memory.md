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
in all. The tag says what the payload is, which is what `is` tests read and
what tells the release path whether the payload owns a reference.

A value wider than one word — a `Range`, say — is boxed on the way into an
`Any`. That is what keeps `Any` one size whatever it holds, which is what lets
`List<Any>` have a stride at all.

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

The counts are **not atomic**, because Keal has no threads. The direction is
now chosen: if concurrency comes, it will be **actor-style** — one heap per
thread, messages between them — precisely so that no object ever crosses
threads and the counts can stay plain. Shared-memory threads would have made
every retain and release an atomic operation, a real cost paid everywhere to
enable races nothing in the language could check.

### Cycles leak, for now

Reference counting cannot free a cycle. Today one cannot easily be built:
records are immutable, so a record can only refer to values that already
existed. A class with a `var` field is another matter:

```keal
class Node(var next: Node?)
val a = Node(null)
a.next = a          // a cycle; its memory is never returned
```

The interpreter has the same behaviour, since it is built on Rust's `Rc`, so
this is not a regression — but it is a gap, and it wants a decision rather
than silence. The options, in rough order of cost:

1. **Leave it.** Document it and let programs avoid cycles. Cheapest, and
   defensible for a language whose data is mostly records.
2. **Weak references.** A `weak Node?` that does not keep its target alive,
   which is how Swift handles it. Puts the problem in the author's hands, and
   gives them a tool.
3. **A cycle collector** alongside the counts, which is what CPython does. It
   catches everything, and brings back some of the runtime that counting was
   chosen to avoid.

This is not settled. It does not block the layout work, which is why it has
been recorded rather than resolved.

---

## 6. Crossing into C

`keal layout` marks which representations can be handed to C as they are:

* `Int`, `Float`, `Bool`, `Unit` and `Range` cross cleanly. They are `int64_t`,
  `double`, a byte, nothing, and a pair of `int64_t`.
* A `T?` over one of those crosses too: it is a small struct.
* **A pointer to a counted object does not.** It has the shape C expects but
  not the lifetime — something has to release it, and C does not know how.

That last one is the whole of the interop problem, and it will need the
boundary to say which way ownership moves rather than passing a pointer and
hoping. Declaring that is future work; recognising it is not.

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
