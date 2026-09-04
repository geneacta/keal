fn main() {
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < 100_000_000 { total += i % 7; i += 1; }
    println!("{}", total);
}
