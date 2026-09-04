#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

typedef struct { int64_t *d; size_t n, cap; } List;
static void push(List *l, int64_t v) {
    if (l->n == l->cap) {
        l->cap = l->cap ? l->cap * 2 : 8;
        l->d = realloc(l->d, l->cap * sizeof(int64_t));
    }
    l->d[l->n++] = v;
}
int main(void) {
    List xs = {0};
    for (int64_t i = 0; i < 1000000; i++) push(&xs, i % 1000);

    List doubled = {0};
    for (size_t i = 0; i < xs.n; i++) push(&doubled, xs.d[i] * 2);

    List big = {0};
    for (size_t i = 0; i < doubled.n; i++) if (doubled.d[i] > 1000) push(&big, doubled.d[i]);

    int64_t acc = 0;
    for (size_t i = 0; i < big.n; i++) acc += big.d[i];
    printf("%lld\n", (long long)acc);

    free(xs.d); free(doubled.d); free(big.d);
    return 0;
}
