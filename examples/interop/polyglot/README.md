# One file, six languages

`main.keal` calls **C, C++, Rust, Go, Java and Kotlin**. No FFI framework:
the native four meet Keal at the C ABI it compiles to, and the JVM two ride
the gateway behind a no-path `import`.

```sh
./run.sh
```

which is only six honest build lines:

| Language | Contribution | How it crosses |
|---|---|---|
| C | `native.c` | handed to `keal build` as a source |
| C++ | `native.cpp` (extern "C") | same — each source gets its own compiler |
| Rust | `../rust` staticlib | `cargo build --release`, linked as `.a` |
| Go | `../go` c-archive | `go build -buildmode=c-archive`, linked as `.a` |
| Java | the JDK itself | `import java.util.UUID` (jbind generates on the spot) |
| Kotlin | `kotlin/Greeter.kt` | `kotlinc -include-runtime` jar on the classpath, `import GreeterKt` |

One `keal bindgen poly.h > bindings.keal` covers all four native
languages — the header spells every boundary in the exact C types Keal
binds, `extern "C"` guard included. Verified output:

```
42
C PLUS PLUS!
6765
hello from Rust, Keal
5.0
GO!
123e4567-e89b-12d3-a456-426614174000
1
KOTLIN!
6765
```

(Both `6765`s are fib(20) — one computed by Rust, one by Kotlin.)
