# Interop: calling C, C++, Rust, Go, Java and Kotlin from Keal

Keal compiles to a single C11 translation unit. That one fact is the
foundation of this whole plan: **the C ABI is the lingua franca of every
runtime worth talking to**. Each language below either speaks it natively
(C, C++, Rust), can be told to (Go), or exposes a C doorway into its virtual
machine (Java, Kotlin via JNI). So the path is not six bridges — it is one
bridge, widened in stages, plus one gateway per foreign runtime.

This document is the plan: what exists, what each stage adds, what it costs,
and in what order to build it.

---

## Where we stand (tier 0 — shipped)

Working today, tested by the suite (`extern_programs_build_and_run`):

```keal
native """
#include <math.h>
double hypot3(double a, double b, double c) { return sqrt(a*a + b*b + c*c); }
"""

extern fun hypot3(a: Float, b: Float, c: Float): Float

println(hypot3(1.0, 2.0, 2.0))   // 3.0
```

* `native "..."` — C pasted verbatim into the generated translation unit.
* `extern fun name(...): Ret [= "symbol"]` — a C symbol made callable, with
  the checker holding callers to the declared signature.
* `keal build prog.keal extra.c extra.cpp` — extra C/C++ sources compiled
  and linked in; any C++ among them switches the linker to `c++` so its
  runtime is present. **This is already C++ interop** for anything wrapped
  in an `extern "C"` function.
* The boundary is deliberately narrow: only `Int`, `Float` and `Bool`
  cross, because they carry no ownership (docs/memory.md §6).

Everything below widens that boundary or drives it from the other side.

---

## Tier 1 — a richer C boundary (the enabler for everything else)

Every other tier stands on this one. Three pieces, in order:

**1a. Strings across the boundary.** The ownership rule the memory model
already dictates, made explicit in the signature:

```keal
extern fun parse(source: borrow String): Int      // C reads, does not keep
extern fun render(doc: Int): own String           // C hands us a malloc'd buffer
```

* `borrow String` passes `const char*` + length; the callee must not retain.
* `own String` on a return means Keal adopts the buffer and frees it.
* Cost: small — two calling conventions in the emitter, a `keal_str_adopt`
  in the runtime. No new syntax beyond the two modifiers.

**1b. Structs across the boundary.** A Keal `record` whose fields are all
C-compatible (`Int`, `Float`, `Bool` — the layout table already knows) may
be declared `@repr(c)` and passed **by value** as the matching C struct.
`keal layout` already prints exactly the struct C will see; this stage just
allows it through `extern`.

**1c. Header generation: `keal emit-header prog.keal > prog.h`.** The
reverse direction — C calling *into* Keal. Every `fun` whose signature fits
the boundary gets a prototype; the generated C already has stable mangled
names (`k_name`). This is what makes Keal usable as a *library* language,
and it is a prerequisite for the Go and JVM gateways, which need to call
back.

**1d. Callbacks.** An `extern fun` parameter of function type passes a C
function pointer built from a Keal closure (the closure header already
carries the code pointer; the missing piece is a trampoline per signature).

*Effort: the largest single item here is 1a; each of 1b–1d is a contained
emitter feature. All are testable by the existing byte-equality discipline —
oracle first, twin mirrored.*

---

## Tier 2 — Rust (near-free once tier 1 lands)

Rust exports the C ABI natively: `#[no_mangle] extern "C" fn` on their side,
`extern fun` on ours, `keal build prog.keal --lib target/release/libx.a`
on the command line. Two additions carry it:

* **Link inputs**: let `keal build` accept `.a`/`.so`/`.dylib` and
  `-l`/`-L` flags, passed through to the linker. Trivial.
* **`keal bindgen header.h`**: a generator that reads a C header and writes
  the `extern fun` declarations — mechanical once tier 1 fixes the type
  mapping. Rust crates expose headers via `cbindgen`, so the chain is:
  `cbindgen` → `.h` → `keal bindgen` → `.keal` declarations. The same tool
  serves plain C libraries (sqlite, curl, ...), which makes it the highest
  -leverage item in this whole document.

*Rust's ownership marries cleanly with tier 1a: `borrow` ↔ `&str`/`&[u8]`,
`own` ↔ `Box`/`CString::into_raw`. No runtime is embedded; the cost is one
static library in the link line.*

---

## Tier 3 — Go (one gateway, some weight)

Go compiles to a C-linkable archive: `go build -buildmode=c-archive`
produces `libx.a` **plus a generated header** — which `keal bindgen` from
tier 2 already consumes. So the mechanism is exactly the Rust path. What is
different is the cost, and it should be stated honestly:

* The Go **runtime rides along** (goroutine scheduler, GC — megabytes, and
  it spawns threads on load).
* Values crossing must be copied: Go's GC may move nothing today, but its
  pointers must not be retained by C — so strings/slices cross as copies,
  which tier 1a's `borrow`/`own` conventions express already.
* Callbacks into Keal work via the tier 1c header + cgo's `//export`.

*Verdict: supported by the same two tools (link inputs + bindgen); document
the weight, add one worked example under `examples/interop/go/`.*

---

## Tier 4 — Java and Kotlin (a gateway into the JVM)

Kotlin/JVM and Java are the same target: **the JVM**, reached through JNI —
a C API (`jni.h`) that the generated C can call directly, because the
generated program *is* C. No new backend capability is needed beyond tier 1;
what is needed is a runtime module and a wrapper generator.

**4a. The JVM host module** (`native`/`extern` over `libjvm`):

```keal
import "std/jvm.keal"

jvmStart(["-Djava.class.path=lib/commons.jar"])
val cls = jvmClass("java/util/UUID")
val id = jvmCallStatic(cls, "randomUUID", "()Ljava/util/UUID;", [])
println(jvmToString(id))   // e.g. 3f2a...-...
```

* `JNI_CreateJavaVM` behind `jvmStart`; object handles are opaque Keal
  classes wrapping `jobject` global refs, released by the ordinary
  reference-counting `release` (the class's release calls `DeleteGlobalRef`
  — the memory model's "count first, then fields" makes this a one-liner).
* `Int`/`Float`/`Bool`/`String` map to `jlong`/`jdouble`/`jboolean`/
  `jstring` (strings copied at the boundary, tier 1a rules).
* Exceptions: after every JNI call, `ExceptionOccurred` → a Keal panic
  carrying the Java message. Honest and simple first; typed errors later.

**4b. `keal jbind Foo.jar com.example.Api`** — the wrapper generator. It
reads class files (or runs `javap`) and emits typed Keal wrappers:

```keal
// generated
class JUuid(val handle: JvmRef) {
    fun toString(): String { return jvmToString(this.handle) }
}
fun uuidRandom(): JUuid { ... }
```

so user code never touches JNI signatures. Kotlin classes are plain JVM
classes — same generator, Kotlin's stdlib on the classpath. This is the
"import Java/Kotlin classes" experience: `jbind` at build time, typed Keal
API at write time.

**4c. Later, if wanted:** GraalVM `native-image --shared` turns a JVM
library into a plain C shared library with its own header — then the JVM
disappears entirely and the *Rust path* carries Java code too. Worth a
worked example once 4a exists; not worth building first, because it
constrains which libraries work (no dynamic class loading).

*Cost to state plainly: the JVM gateway embeds a JVM (or Graal image); it
is for programs that need Java libraries, not a default. The determinism
story of the test suite stops at the boundary — JVM output is the JVM's.*

---

## Cross-cutting rules (all tiers)

* **Ownership is written down, never guessed** — `borrow`/`own` at the
  boundary, exactly as docs/memory.md §6 demands.
* **Match-or-refuse holds at the boundary too**: a signature the emitter
  cannot represent is refused by name at compile time, never truncated.
* **The suite discipline extends**: every tier lands oracle-first in
  `src/cbackend.rs`, is mirrored in `selfhost/cbackend.keal`, and the
  byte-equality tests plus a worked example under `examples/interop/`
  keep it honest. The bootstrapped `dist/kealc` gets each feature the same
  day the Rust oracle does, or the fixed-point test fails.
* **One manifest, eventually**: once link inputs exist, a `keal.toml`
  listing sources, libraries and flags replaces the growing command line.

## Suggested order

| # | Item | Unlocks |
|---|------|---------|
| 1 | Tier 1a strings + 1b structs | everything |
| 2 | Link inputs (`.a`, `-l`) | Rust, Go, C libraries |
| 3 | `keal bindgen` for C headers | sqlite/curl/every C lib, Rust via cbindgen, Go via c-archive |
| 4 | Tier 1c `emit-header` + 1d callbacks | Keal as a library; Go/JVM callbacks |
| 5 | JVM host module (4a) | Java + Kotlin |
| 6 | `keal jbind` (4b) | typed Java/Kotlin imports |

Each row is independently shippable and independently testable; nothing in a
later row forces rework of an earlier one.
