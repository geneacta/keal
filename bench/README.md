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

These are ordinary Keal programs, so they double as tests: the suite checks
that both engines print the same thing for each of them.
