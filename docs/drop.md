# Unwinding through reference counts — the design behind native `try`

Keal panics are catchable (`try`/`catch`) on all three engines. The two
interpreters unwind trivially — Rust's `?` releases everything it passes.
The C backend was the hard case: reference counting means every skipped
scope holds references someone must release, and "match or refuse" means
a caught panic must behave — and print — exactly like the VM, with zero
leaks even on the unwind path. This file records the design that landed
and the ones it beat.

## Rejected: `setjmp`/`longjmp`

The classic shape: `try` does `setjmp`, `keal_panic` does `longjmp`.
Rejected because a `longjmp` skips every C frame between throw and catch
without releasing what those frames own. Fixing that needs either a
runtime shadow stack of live references (a cost on every function, paid
by programs that never `throw`) or C++-style unwind tables (a compiler
project of its own). Both lose to the chosen design on simplicity and on
the zero-cost-when-unused rule.

## Rejected: error-return codes everywhere

Compile every function to return a status alongside its value. Correct,
but it taxes every call in every program forever, and it contorts the
generated C the backend works hard to keep readable.

## Chosen: checked unwinding (poisoned returns)

Three cooperating pieces, **all emitted only when the program contains a
`try` at all** — `program_has_try` scans the AST, and a program without
one compiles byte-for-byte as before, paying nothing:

1. **The runtime records instead of exiting.** `keal_try_depth` counts
   active `try` blocks. With none active, `keal_panic` prints and exits
   exactly as always. With one active, it sets `keal_unwinding`, stores
   the message, and *returns*; every runtime helper that can panic
   returns a harmless poison right after (`0`, `NULL`, an empty word),
   so nothing dereferences garbage.

2. **The generated code checks after anything that can panic.** Every
   call, arithmetic helper, index, `!!` and `assert` is a statement-level
   temp already (the backend's normal shape), and each is followed by
   `if (keal_unwinding) { goto <label>; }` — placed before the value is
   consumed and before any observable effect, so output stays identical
   to the interpreters up to the instant of the panic.

3. **Every block knows how to leave.** Each scope carries an unwind
   label that releases everything the block *ever* owned — counted
   declarations hoist to the top of their block as `T x = NULL;`, so the
   label is safe on every path — then chains to the enclosing scope's
   label. A function's bottom label returns a poison value; the caller's
   check catches the flag and keeps unwinding. A `try`'s body chains to
   its catch label instead, which decrements the depth, adopts the
   message (`keal_unwind_take`), and runs the handler. Frame by frame,
   scope by scope, every reference is released by code that knows its
   name — no tables, no jumps over live data.

The subtleties the tests pin down:

* **`return` transfers ownership out of a `try`** — `disown` thins the
  normal release list, but the unwind label keeps the *ever owned* list,
  because a check earlier in the block still needs those released.
* **Ownership transferred into a panicking call** (`l[i] = v`,
  `insert`) is owned by a temp first and NULLed only after the call
  proves clean, so a bounds panic cannot strand the retained reference.
* **Constructors zero their instance** (`memset`) before running field
  initializers, so a panic mid-construction releases a half-built object
  safely.
* **JNI exceptions ride the same rails**: `keal_jvm_check` returns
  whether Java threw, every gateway helper bails right after, and a
  `DateTimeException` lands in a Keal `catch` inside a native binary.

Cost, stated honestly: in a program that uses `try`, every panicking
operation gains one well-predicted branch, counted declarations lose
`const` and initialize twice, and releases on the normal path are
unchanged. In a program that does not, the emitted C is identical to
what it was before this design existed. What native `try` still does not
catch: C stack exhaustion (the VM's `MAX_DEPTH` panic is catchable, a
native segfault is not).

## The other half: `deinit` — shipped

The hook landed, named `deinit` (`drop` already belongs to the
take/`drop` pair on sequences and strings): a class or record may declare
`proc deinit()`, and it runs when the object's last reference dies.
`keal jbind`'s wrappers use it to free their JVM handles by themselves —
`free()` stays as the manual lever, idempotent either way.

The semantics, stated exactly:

* **When.** A death is *queued*; the queue drains at the next statement
  boundary, cascades included (an object freed by a `deinit` joins the
  same sweep). A statement that leaves early — `return`, `break`,
  `throw` — drains at the boundary it lands on, unwinding included.
* **Order.** Reverse-declaration order — the destructor convention —
  everywhere: the interpreter's scopes tear down youngest-first (a
  `HashMap` would tear down in hash order, so scopes remember their
  insertion order), the VM pops frame locals youngest-first and clears
  block slots in reverse, the C backend always released in reverse.
* **Once.** `deinit` runs at most once per object, marked before it is
  queued. If the body stores `this` somewhere, the object survives —
  resurrected, never re-dropped. Calling `deinit` yourself is a checker
  error; give the class an ordinary method for manual release.
* **What it costs.** Everything is gated on the program declaring one:
  without a `deinit`, the three engines compile and run byte-for-byte
  as before. With one, every statement gains a drain call, VM locals are
  cleared at block ends, and the C backend releases each statement's
  expression temporaries at its boundary (they otherwise live to the
  block's end, which would postpone the hook).
* **The stated limits.** Objects still alive when the program ends do
  not `deinit` — like every finalizer since finalizers. An object cycle
  never dies, so it never `deinit`s (docs/memory.md §5). And `keal
  layout` does not yet show the one-byte `kdropped` flag a deinit-class
  carries natively.
