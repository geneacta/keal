fun main() {
    var total = 0L
    var i = 0L
    while (i < 100000000L) { total += i % 7; i += 1 }
    println(total)
}
