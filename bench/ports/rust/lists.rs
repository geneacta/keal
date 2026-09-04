fn main() {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..1_000_000i64 { xs.push(i % 1000); }
    let doubled: Vec<i64> = xs.iter().map(|it| it * 2).collect();
    let big: Vec<i64> = doubled.iter().filter(|&&it| it > 1000).cloned().collect();
    let acc: i64 = big.iter().fold(0i64, |acc, n| acc + n);
    println!("{}", acc);
}
