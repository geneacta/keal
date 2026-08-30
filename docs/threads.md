# Actors on real threads — decided, then done

The actor model shipped deterministic (prelude `ActorSystem`: mailboxes,
`spawn`, round-robin `run`). This file decided how it would reach real
threads without breaking the language's promises — and now records that
it has: `keal build` runs every actor on its own OS thread. The API
never changed — that was the point of freezing it.

## The load-bearing insight

`run` promises delivery order **within one actor's mailbox** and nothing
between actors. That makes the deterministic round-robin a *legal
schedule* — one of many. So threading is not a semantics change; it is a
scheduler that picks different legal schedules:

* The interpreters may keep running the deterministic schedule forever
  and remain correct. Nothing forces three engines to interleave alike,
  because interleaving was never observable in a well-formed program.
* The byte-for-byte test discipline continues to hold for programs whose
  output does not depend on cross-actor ordering — which is exactly the
  class of programs the actor model tells you to write. Order-sensitive
  output under threads is a bug in the program, not in the engine.

## What the interpreters can and cannot do

Handler closures capture `Rc` environments; Keal values in the Rust
engines are `Rc` + `RefCell`, deliberately `!Send` — the memory model's
"counts stay plain" promise is that same decision seen from C. Moving a
closure to another OS thread would need `Arc`/`Mutex` values everywhere:
a ground-up rewrite that taxes every program to speed up a few. **Decided:
the interpreters keep the deterministic scheduler.** They are the
semantic reference, and the deterministic schedule is legal.

Real threads are the native backend's job, where the compiler already
monomorphizes and the runtime is ours.

## The native design

* **One heap per actor.** An object never crosses threads; only
  *messages* do, and a message crosses by **deep copy** into the
  receiving actor's world. The backend monomorphizes `ActorSystem<M>`,
  so it can generate `copy_M` per message type. Counts stay plain in a
  program without actors; with them, counts go atomic (one `#define` in
  the generated C) — because addresses, the strings a copy shares, and
  immutable globals *are* visible from two threads, and who-frees-last
  needs an atomic answer. docs/memory.md states the trade.
* **Message types must be copyable.** `M` may hold Int/Float/Bool/String,
  records, lists and maps of these — data. Closures, cells and actor
  handles inside `M` are refused **by name at compile time** (the
  match-or-refuse rule; `ActorRef` itself is the one blessed exception,
  carried as its mailbox id). This restriction is honest and permanent:
  a closure is an environment, and environments do not cross heaps.
* **Runtime shape.** The actor classes stay ordinary compiled Keal;
  exactly four method bodies are generated instead of compiled, per
  monomorphization — they *are* the scheduler. `send` and `post`
  deep-copy outside the one actor lock (the copy reads only the
  sender's values) and enqueue under it; `drain` snapshots under it,
  as copies; `run` starts one OS thread per actor, waits on a condition
  until every mailbox is empty with no handler in flight, and joins. A
  handler's panic is carried back in the run state and rethrown on the
  thread that called `run`, so `try { sys.run() }` catches it there on
  every engine; without a `try` in the program the actor thread ends
  the process at the panic site, message and line intact. `try` inside
  a handler works as anywhere — the unwind state is per-thread, and so
  is the `deinit` queue, drained on the actor's own thread.
* **Groundwork already landed:** the unwind flags (`keal_try_depth`,
  `keal_unwinding`, message buffer) and the `deinit` queue are
  `_Thread_local`, so every thread panics, catches and deinits
  independently today. (The interpreters' queues were thread-local from
  day one.)

## Staging

1. **Groundwork** — thread-local runtime state. *Done.*
2. **`copy` + the message-safety check.** *Done, on all three engines:*
   `copy(value)` deep-copies data everywhere — the checker refuses what
   cannot cross (functions, `Any`, open type parameters), a cyclic value
   is refused at run time with the same depth-cap panic on every engine,
   and the C backend generates one copy function per type (lists, maps,
   classes, nullables — recursive types memoize; the unwind path
   releases the partial copy, so a caught cycle panic leaks nothing).
   This is the scheduler's `copy_M`, landed early and user-facing.
3. **The capture semantics — SHIPPED.** `spawn` copies its handler's
   captures per actor (`copyClosure`, a builtin all three engines
   implement: the interpreter rebuilds the environment from the same
   free-variable analysis the compilers use, the VM re-cells, the C
   backend generates a per-lambda `_copy` for the closure header's new
   `copy` slot). `send` copies every message. The checker holds the
   whole rule set at the spawn site: the handler is written in place,
   captures must be copyable data, `this` cannot come along, and a
   handler may not reach a global `var` or a mutable global `val` —
   the addresses (`ActorRef`, and the new `Outbox<T>`, main's own
   mailbox for results) are blessed shared exceptions. Programs
   aggregate the actor way now: state in a local the actor copies,
   results out through an `Outbox`, reply addresses inside messages.
   Two real bugs fell out of building it: `Nullable(ActorRef)` slipped
   past the address blessing and duplicated mailboxes, and — a
   pre-existing one — empty container literals in *generic* class
   bodies kept their `List<Never>` type (the expected type's rigid
   params were mistaken for inference variables), so every generic
   field's native list carried a NULL release thunk and leaked its
   elements. Both fixed, both pinned by tests.

4. **The capture wall, hit early and settled.** Building toward the
   runtime exposed the real wall, and it was not the pthreads: **handler
   closures shared captured state.** Two handlers could capture the same
   list and both mutate it — legal and deterministic under the
   round-robin, a data race under threads, and no lock can hide it.
   The decision, adopted by all three engines *together* in stage 3:
   **`spawn` copies its handler's captured values, per actor** — an
   actor's state is its own, full stop, and aggregation happens the
   actor way: replies inside messages, results through an `Outbox`.
   The checker holds captured values to the same copyability rule
   messages obey.
5. **The scheduler itself — SHIPPED.** As promised, a pure runtime
   project: the semantics above needed no change. The generated C
   defines `KEAL_ACTORS`, under which the runtime's counts go atomic
   and one mutex/condvar pair exists; four generated method bodies
   (`send`, `post`, `drain`, `run` — see "Runtime shape") put every
   actor on its own OS thread with quiescence as the join. Verified the
   way a scheduler has to be: the suite's mesh program — eight actors
   fanning echoes at each other into one outbox — runs under
   **ThreadSanitizer**, five times, clean, and every actor test still
   prints identical bytes on all three engines, leak-free. What panics
   do, `deinit` on the actor's thread, and `try { sys.run() }` are
   pinned by `tests/native/actor-panics.keal`.
6. **Measure before optimizing** — measured (Apple M4, 10 cores,
   `-O2`; self-send chains, medians of repeated runs):
   * *Compute-bound scaling*: 320M spins of arithmetic split over 8
     actors ran in 0.14s wall against 0.84s on one actor — **~6×
     faster on 8**, so the global lock does not gate handlers that do
     real work.
   * *Raw message cost*: 400k messages through one actor in 0.02s —
     about **50ns a message** (copy, lock, push, broadcast, deliver).
     The same 400k spread over eight actors took 0.07s wall with the
     time gone to `sys` — the every-completion **broadcast wakes every
     thread**, and that storm is the scheduler's one visible cost.
   * Verdict: millions of messages a second and near-linear compute
     scaling; **nothing to optimize yet**. If a real program ever
     drowns in that sys time, the first lever is known and bounded:
     a condvar per mailbox instead of one broadcast for all.

JNI note, closed: a JNIEnv is only valid on the thread it was handed
to, so the gateway keeps one per thread — the first JVM call an actor
makes attaches its thread lazily, a pthread-key destructor detaches it
when the actor ends, and the argument buffer went thread-local with it.
`examples/interop/java/actordate.keal` is an actor asking `java.time`
for weekdays from its own thread; the suite runs it under a JDK.

*Cost, as it turned out: stages 1–5 changed no line of user code. The
scheduler is four generated method bodies and one runtime section behind
a `#define` — in a program without actors the define is absent, the
count macros collapse to plain `++`/`--`, and no pthread is included.
That claim was checkable, so it got checked, and the first check failed:
the switch scanned the whole program for the actor names, and the
**prelude declares them**, so every program was compiling with atomic
counts. Declaring is not using — the file that declares `ActorSystem` is
now excluded from the scan, and `grep KEAL_ACTORS` on the emitted C is
the one-line audit. (The bug cost about 15% on a refcount-saturated
microbenchmark and nothing measurable elsewhere; it was wrong regardless.)*
