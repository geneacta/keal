# Actors on real threads — the plan, decided

The actor model shipped deterministic (prelude `ActorSystem`: mailboxes,
`spawn`, round-robin `run`). This file decides how it reaches real
threads without breaking the language's promises, and what each stage
costs. The API does not change — that was the point of freezing it.

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

* **One heap per actor, counts stay plain.** An object never crosses
  threads; only *messages* do, and a message crosses by **deep copy**
  into the receiving actor's world. The backend monomorphizes
  `ActorSystem<M>`, so it can generate `copy_M` per message type.
* **Message types must be copyable.** `M` may hold Int/Float/Bool/String,
  records, lists and maps of these — data. Closures, cells and actor
  handles inside `M` are refused **by name at compile time** (the
  match-or-refuse rule; `ActorRef` itself is the one blessed exception,
  carried as its mailbox id). This restriction is honest and permanent:
  a closure is an environment, and environments do not cross heaps.
* **Runtime shape.** `KealActor { pthread_t, mutex, condvar, deque }`;
  `send` locks, copies, signals; each actor thread loops on its mailbox;
  `run` becomes the join point (start threads, wait until every mailbox
  is empty and every actor idle, stop them). Panics in an actor carry to
  `run` and end the program with the actor named — `try` inside a
  handler works as anywhere (the unwind state is per-thread).
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
3. **The capture decision, then the scheduler.** Building the runtime
   exposed the real wall, and it is not the pthreads: **handler closures
   share captured state.** Today two handlers may capture the same list
   and both mutate it — legal and deterministic under the round-robin,
   a data race under threads, and no lock can hide it (plain counts,
   mutable containers). The decision, recorded now so the scheduler can
   be built against it: **`spawn` will copy its handler's captured
   values, per actor** — an actor's state is its own, full stop, and
   aggregation happens the actor way, by messages (a `Report` reply, a
   collector actor), not through a shared list. This is a semantic
   tightening that all three engines adopt *together* — the
   deterministic engines start copying captures in the same change as
   the threaded native, so programs never behave differently by engine
   — and the checker enforces that captured values satisfy the same
   copyability rule messages do. The existing actor tests aggregate
   through shared captures and will be rewritten to reply-patterns as
   part of that change. Then the scheduler itself: `KealActor`
   (pthread, mutex, condvar, deque), `send` locks-copies-signals,
   `run` joins, TSan in the suite.
   *Not started; the copy machinery it needs is now fully in place.*
4. **Measure before optimizing** — per-actor arenas only if malloc
   contention shows up in real programs.

JNI note for later: a JVM call from an actor thread needs
`AttachCurrentThread`; the gateway will attach lazily per thread.

*Cost stated plainly: stage 2 is checker + backend work of moderate
size; stage 3 is the real project — a careful C runtime with locks,
joins and panic paths, verified under a thread sanitizer. Nothing in
either changes a line of user code.*
