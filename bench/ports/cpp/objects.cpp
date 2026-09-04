#include <cstdio>
#include <cstdint>
#include <cstdlib>
struct Point { std::int64_t x, y; };
static std::int64_t manhattan(const Point &p) { return std::llabs(p.x) + std::llabs(p.y); }
int main() {
    std::int64_t sum = 0;
    for (std::int64_t i = 0; i < 10000000; i++) {
        Point p{ i % 100 - 50, i % 37 - 18 };
        sum += manhattan(p);
    }
    std::printf("%lld\n", (long long)sum);
    return 0;
}
