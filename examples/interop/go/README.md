# Calling Go from Keal

Same road as Rust: the C ABI. Go exports it with cgo's `c-archive` build
mode; Keal's output *is* C. Four commands, no glue code written by hand.

```sh
# 1. A Go static archive that exports //export functions.
go build -buildmode=c-archive -o libgodemo.a godemo.go

# 2. Keal bindings from the C header.
keal bindgen godemo.h > bindings.keal

# 3. Build the program against the archive.
keal build main.keal libgodemo.a -I.
./main
```

One honest note on the header: cgo also generates `libgodemo.h`, but it
spells the boundary in Go's typedefs (`GoInt64`, plain `char*`), which
`bindgen` deliberately refuses to guess about. `godemo.h` is the same ABI
written in the exact C types Keal binds — `int64_t`, `double`, `bool`,
`const char*` in (borrowed; `C.GoString` copies), `char*` out (owned;
`C.CString` allocates with the C allocator, which is exactly what Keal
frees). Five prototypes, written once.

Verified output. macOS `leaks` reports exactly one 48-byte allocation,
inside the Go runtime's own startup (`runtime.osinit_hack`, an xpc date
object) — none from Keal:

```
6765
5.0
true
Go c-archive reporting
KEAL CALLS GO!
```
