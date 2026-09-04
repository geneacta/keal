# Benchmarks

Four programs, each chosen for a different cost:

| | what it stresses |
|---|---|
| `fib.keal` | call overhead and integer arithmetic |
| `loops.keal` | tight loops and variable traffic |
| `objects.keal` | allocation, field reads, method calls |
| `lists.keal` | collections and closures |

Run both engines and compare:

```sh
cargo build --release
for f in bench/*.keal; do
  echo "$f"
  time ./target/release/keal --ast "$f" >/dev/null
  time ./target/release/keal --vm  "$f" >/dev/null
done
```

These are ordinary Keal programs, but nothing checks them: `cargo test
--release` walks `tests/programs` and `examples`, and never this directory.
A change that made the engines disagree here would not turn the suite red.
Copy a program into `tests/programs` if you want that guarantee for it.
