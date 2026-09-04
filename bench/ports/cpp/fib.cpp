#include <cstdio>
#include <cstdint>
static std::int64_t fib(std::int64_t n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}
int main() { std::printf("%lld\n", (long long)fib(35)); return 0; }
