fun main() {
    val xs = ArrayList<Long>()
    for (i in 0 until 1000000L) { xs.add(i % 1000) }
    val doubled = xs.map { it * 2 }
    val big = doubled.filter { it > 1000 }
    println(big.fold(0L) { acc, n -> acc + n })
}
