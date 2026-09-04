#include <stdio.h>
#include <stdint.h>
typedef struct { int64_t x, y; } Point;
static int64_t iabs(int64_t v) { return v < 0 ? -v : v; }
static int64_t manhattan(Point p) { return iabs(p.x) + iabs(p.y); }
int main(void) {
    int64_t sum = 0;
    for (int64_t i = 0; i < 10000000; i++) {
        Point p = { i % 100 - 50, i % 37 - 18 };
        sum += manhattan(p);
    }
    printf("%lld\n", (long long)sum);
    return 0;
}
