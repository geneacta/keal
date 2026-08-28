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
2. **`copy_M` generation + message-safety check** — the checker's
   structural "crosses threads" predicate on `M`, and the generated deep
   copy. Testable single-threaded (copy, mutate, compare).
3. **The pthread scheduler behind the same `run()`** — gated like
   everything else: a program that never spawns pays nothing. Verified
   with order-insensitive programs (counters, joined sets) on all three
   engines, plus native stress runs under TSan.
4. **Measure before optimizing** — per-actor arenas only if malloc
   contention shows up in real programs.

JNI note for later: a JVM call from an actor thread needs
`AttachCurrentThread`; the gateway will attach lazily per thread.

*Cost stated plainly: stage 2 is checker + backend work of moderate
size; stage 3 is the real project — a careful C runtime with locks,
joins and panic paths, verified under a thread sanitizer. Nothing in
either changes a line of user code.*
