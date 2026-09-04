#include <stdio.h>
#include <stdint.h>
int main(void) {
    int64_t total = 0;
    int64_t i = 0;
    while (i < 100000000) { total += i % 7; i += 1; }
    printf("%lld\n", (long long)total);
    return 0;
}
