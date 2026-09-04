data class Point(val x: Long, val y: Long)

fun manhattan(p: Point): Long = Math.abs(p.x) + Math.abs(p.y)

fun main() {
    var sum = 0L
    for (i in 0 until 10000000L) {
        val p = Point(i % 100 - 50, i % 37 - 18)
        sum += manhattan(p)
    }
    println(sum)
}
