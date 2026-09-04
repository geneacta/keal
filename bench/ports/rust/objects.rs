struct Point { x: i64, y: i64 }
fn manhattan(p: &Point) -> i64 { p.x.abs() + p.y.abs() }
fn main() {
    let mut sum: i64 = 0;
    for i in 0..10_000_000i64 {
        let p = Point { x: i % 100 - 50, y: i % 37 - 18 };
        sum += manhattan(&p);
    }
    println!("{}", sum);
}
