# Calling Rust from Keal

The whole path is the C ABI: Rust exports it natively, Keal's output *is* C.
Four commands, no glue code written by hand.

```sh
# 1. A Rust static library that exports extern "C" functions.
cargo build --release            # produces target/release/libkealdemo.a

# 2. Its C header, straight from the annotations.
cbindgen --lang c --output kealdemo.h

# 3. Keal bindings, straight from the header.
keal bindgen kealdemo.h > bindings.keal

# 4. Build the program against the archive.
keal build main.keal target/release/libkealdemo.a -I.
./main
```

`bindgen` binds exactly what crosses — `i64`/`f64`/`bool`, `*const c_char`
as `borrow String`, a returned `CString::into_raw` as `own String` (Keal
frees it; allocate with the C allocator) — and skips the rest with the
reason printed. On Linux add `-lpthread -ldl` to the build line, which the
Rust runtime needs.
