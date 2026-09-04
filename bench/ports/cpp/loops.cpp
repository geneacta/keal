#include <cstdio>
#include <cstdint>
int main() {
    std::int64_t total = 0, i = 0;
    while (i < 100000000) { total += i % 7; i += 1; }
    std::printf("%lld\n", (long long)total);
    return 0;
}
