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

## What the C boundary needs on Windows

The generated C is C11 and portable, with one exception that decides the
toolchain: overflow is checked with `__builtin_mul_overflow` and its
siblings, which are GCC and Clang builtins. **MSVC cannot compile the
runtime**, and a default Rust install on Windows brings exactly MSVC — so
the compiler will build there and `keal build` will not, until a second
toolchain is installed.

Install **MinGW-w64 in a POSIX-threads flavour** (the `win32` and `mcf`
flavours ship no `pthread.h`, and the actor scheduler needs one), or LLVM
clang targeting mingw32. `keal build` looks for `CC`, then `cc`, `gcc`,
`clang`, and takes the first that answers; `keal doctor` names the one it
found. Where the only compiler present is `cl.exe`, the error says that
rather than reporting no compiler at all — it is the difference between a
puzzle and an instruction.

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

Every other tier stands on this one. **Status: 1a, 1b and 1c are shipped**;
1d is deferred, with the reason recorded below.

**1a. Strings across the boundary — SHIPPED.** The ownership rule the
memory model already dictates, explicit in the signature:

```keal
extern fun parse(source: borrow String): Int      // C reads, does not keep
extern fun render(doc: Int): own String           // C hands us a malloc'd buffer
```

* `borrow String` passes the NUL-terminated `const char*`; the callee must
  not retain it past the call.
* `own String` on a result adopts the buffer: Keal counts it and `free()`s
  it with the string (`keal_str_adopt` in the runtime; a NULL from C reads
  as the empty string).
* The checker demands a mode on every `String` crossing, in both
  directions, and rejects a mode anywhere else — misuse is a checked error
  with a note saying what to write.

**1b. Structs across the boundary — SHIPPED.** A `record` whose fields are
all `Int`, `Float` or `Bool` crosses **by value**, no annotation needed: the
generated C defines a headerless mirror `typedef struct Keal_Name { ... }`
with unmangled field names, before any `native` block, and the boundary
copies fields both ways. The C side just writes functions over `Keal_Name`:

```keal
record Vec2(val x: Float, val y: Float)
native """
static double vec2_dot(Keal_Vec2 a, Keal_Vec2 b) { return a.x*b.x + a.y*b.y; }
"""
extern fun vec2_dot(a: Vec2, b: Vec2): Float
```

**1c. Header generation — SHIPPED.** `keal emit-header prog.keal > prog.h`
prints the C face of the boundary: the `Keal_Name` mirror structs (same
text as the generated C, so the two translation units agree) and a `k_name`
prototype for every non-generic function whose signature crosses cleanly.
A companion `.c` file compiled by `keal build prog.keal helper.c` includes
it and calls straight back into Keal — the suite's boundary test does
exactly that. This is the prerequisite the Go and JVM gateways needed.

**1d. Closure callbacks — deferred, deliberately.** Passing a Keal
*closure* to C as a bare function pointer needs somewhere to put the
environment, and C callback APIs differ on it: most take a `void* userdata`
alongside the pointer, some take nothing. Picking a convention shapes the
syntax (`extern fun onEach(f: (Int) -> Int)` must say which argument is the
userdata slot), so it waits for a real consumer instead of guessing.
Meanwhile the shipped 1c covers the common need from the other side: C code
can call named Keal functions (`k_name`) directly, today.

*What shipped landed the way everything lands here: oracle first, the three
self-hosted twins mirrored, byte-equality over the corpus, a build-and-run
boundary test with `leaks` clean, and the bootstrap fixed point re-proven.*

---

## Tier 2 — Rust — SHIPPED

Rust exports the C ABI natively, and both tools it needed exist now:

* **Link inputs — shipped.** `keal build prog.keal libx.a -lm -L/opt/lib`
  passes `.a`/`.so`/`.dylib`/`.o` files and `-l`/`-L` flags to the link
  step, and `-I`/`-D` to the compile steps.
* **`keal bindgen header.h` — shipped.** Reads a C header, writes the
  `extern fun` declarations. It binds only what crosses exactly —
  `int64_t`/`long long`, `double`, `bool`, `const char*` as `borrow
  String`, a returned `char*` as `own String`, `Keal_Name` mirrors as
  records — and **skips everything else with the reason printed**: a
  32-bit `int`, a borrowed `const char*` return, a variadic, a
  function-pointer parameter. A guessed binding is a crash with a delay;
  a skipped one is a wrapper the C author writes in five lines.

The chain is four commands, demonstrated and verified in
[`examples/interop/rust/`](../examples/interop/rust/):

```sh
cargo build --release                      # staticlib with extern "C" exports
cbindgen --lang c --output kealdemo.h      # its header
keal bindgen kealdemo.h > bindings.keal    # its Keal face
keal build main.keal target/release/libkealdemo.a -I.
```

Rust's ownership marries tier 1a exactly: `&CStr` ↔ `borrow String`,
`CString::into_raw` ↔ `own String` (Keal frees it). No runtime is
embedded. The same two tools serve plain C libraries — sqlite, curl —
and headers from `go build -buildmode=c-archive`, which is why tier 3
needs no new machinery.

A convention worth knowing: a hand-written header that defines a mirror
struct should guard it with `#ifndef KEAL_MIRROR_Name`, the same guard the
generated C and `keal emit-header` emit, so the two definitions coexist.

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

**4a. The JVM host module — SHIPPED** ([`lib/jvm.keal`](../lib/jvm.keal)):
one Keal module, no new compiler capability — a `native` block of helpers
over `jni.h` plus `extern fun` declarations riding tier 1's `borrow`/`own`
strings. Verified end to end by the suite (`jvm_gateway_works_end_to_end`,
skipped when no JDK is installed):

```keal
import "lib/jvm.keal"

jvmStart("")                              // or "-Djava.class.path=foo.jar"
val date = jvmClass("java/time/LocalDate")
jvmArgInt(2026)
jvmArgInt(1)
jvmArgInt(1)
val d = jvmStaticObj(date, "of", "(III)Ljava/time/LocalDate;")
jvmArgLong(58)
println(jvmToString(jvmCallObj(d, "plusDays", "(J)Ljava/time/LocalDate;")))
// 2026-02-28 — java.time, in a native Keal binary
```

Build with the JDK on the line (link inputs from tier 2):

```sh
JH=$(/usr/libexec/java_home)
keal build prog.keal -I$JH/include -I$JH/include/darwin \
    -L$JH/lib/server -ljvm -Wl,-rpath,$JH/lib/server
```

The honest v1 shape: objects are opaque `Int` handles (JNI global refs,
freed with `jvmFree`); arguments are pushed with `jvmArg*` matching the
Java parameter type; calls take JNI signatures verbatim; a Java exception
becomes a Keal panic carrying the throwable's `toString`. The worked
example is [`examples/interop/java/`](../examples/interop/java/).

**4b. `keal jbind` — shipped.** The wrapper generator, and the road to
`import java.time.LocalDate`. `keal jbind java.time.LocalDate
java.time.DayOfWeek` reads each class through `javap -public` and prints
one typed Keal module over exactly the 4a calls:

```keal
// generated
class UUID(val handle: Int) : Ord {
    fun toString(): String { return jvmToString(this.handle) }
    proc free() { jvmFree(this.handle) }
    fun compareTo(a0: UUID): Int { ... }
}
fun uuidRandomUUID(): UUID { ... }
```

so user code never touches JNI signatures. The bindgen rule holds: only
members whose types cross exactly are bound — `int`, `long`, `double`,
`boolean`, `String`, and any class bound in the same run (bind `DayOfWeek`
alongside `LocalDate` and `getDayOfWeek()` comes typed) — everything else
is skipped with the reason printed. Statics and constructors become free
functions (`localDateOf`, `uuidNew`); a representable Java `compareTo`
makes the wrapper `Ord`, so prelude `compare` and `<` reach across the
JVM; `free()` releases the global ref (`handle` stays visible for calls
jbind could not type). `--jvm <path>` sets the emitted import path, and an
argument naming a file is read as saved `javap` output — which keeps the
snapshot test (`tests/jbind/`) JDK-free; the end-to-end test builds a
native binary against live-generated `java.time` wrappers. Kotlin classes
are plain JVM classes: same generator, Kotlin's stdlib on the classpath.

The endpoint, in three steps that stack — **all three shipped**:
**(1)** 4a — you write signatures; **(2)** `jbind` — a generated
`LocalDate.keal` you import by path; **(3)** loader sugar —

```keal
import java.time.LocalDate, java.time.DayOfWeek

jvmStart("")
val d = localDateOf(2026, 1, 1).plusDays(58)
println(d.toString())               // 2026-02-28
println(d.getDayOfWeek().toString())  // SATURDAY
```

A non-path import desugars to `.jbind/<classes>.keal` next to the file;
classes named in one import are bound together, so they see each other
typed. `keal run`/`check`/`build` fill the cache through `javap` when it
is missing (`keal jbind --cache .jbind java.time.LocalDate` does it by
hand), and the directory carries its own copy of the gateway, so a
committed `.jbind/` builds anywhere — no JDK needed until the classes
change. The compiler stages themselves never generate: `keal
tokens`/`ast`/`types`/`cgen` and the self-hosted twins read only what is
on disk, which keeps the byte-for-byte corpora meaningful.

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

| # | Item | Unlocks | Status |
|---|------|---------|--------|
| 1 | Tier 1a strings + 1b structs | everything | **shipped** |
| 2 | Tier 1c `emit-header` | Keal as a library; Go/JVM callbacks | **shipped** |
| 3 | Link inputs (`.a`, `-l`, `-L`, `-I`, `-D`) | Rust, Go, C libraries | **shipped** |
| 4 | `keal bindgen` for C headers | sqlite/curl/every C lib, Rust via cbindgen, Go via c-archive | **shipped, with a verified Rust demo** |
| 5 | Closure callbacks (1d) | C APIs that take function pointers | waits for a consumer |
| 6 | JVM host module (4a) | Java + Kotlin | **shipped, with a verified java.time demo** |
| 7 | `keal jbind` (4b) | typed Java/Kotlin imports, then `import java.time.LocalDate` as loader sugar | next |

Each row is independently shippable and independently testable; nothing in a
later row forces rework of an earlier one.
