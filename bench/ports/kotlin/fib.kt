fun fib(n: Long): Long {
    if (n < 2) return n
    return fib(n - 1) + fib(n - 2)
}
fun main() { println(fib(35)) }
