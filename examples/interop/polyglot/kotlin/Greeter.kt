// Kotlin is a JVM language: compiled classes are plain Java classes, so the
// same gateway carries them. Top-level functions land in `GreeterKt`.
fun shout(s: String): String = s.uppercase() + "!"

fun fib(n: Long): Long {
    var a = 0L; var b = 1L
    repeat(n.toInt()) { val t = a + b; a = b; b = t }
    return a
}
