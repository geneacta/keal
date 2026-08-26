/* The runtime the generated C is compiled against.
 *
 * This is the memory model of docs/memory.md made real, for the part of the
 * language the C backend covers: a reference count at the head of every heap
 * object, strings as counted immutable buffers, and checked integer
 * arithmetic so that native code fails where the other two engines fail
 * rather than quietly wrapping.
 *
 * It is emitted into the generated file rather than shipped as a library, so
 * that the output of `keal emit-c` is one self-contained translation unit a
 * C compiler can take without any flags. */

#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* A generated program uses only the part of this runtime it needs, so the
 * rest must not draw warnings from a caller building with -Wall. */
#if defined(__GNUC__) || defined(__clang__)
#define KEAL_FN __attribute__((unused)) static
#else
#define KEAL_FN static
#endif

/* Every heap object begins with its count. A string is that count, a length,
 * and the bytes; `static_bytes` marks a literal, whose bytes are not ours to
 * free. */
typedef struct KealStr {
    int64_t rc;
    int64_t len;
    const char* bytes;
    bool static_bytes;
} KealStr;

KEAL_FN void keal_panic(const char* what, int64_t line) {
    fprintf(stderr, "runtime error: %s\n", what);
    if (line > 0) {
        fprintf(stderr, "  at line %" PRId64 "\n", line);
    }
    exit(1);
}

KEAL_FN void* keal_alloc(size_t n) {
    void* p = malloc(n);
    if (p == NULL) {
        keal_panic("out of memory", 0);
    }
    return p;
}

/* ---- reference counting ---------------------------------------------- */

/* Returns its argument so that a retain can be written inline, which is what
 * lets the generated code read as expressions rather than as statements. */
KEAL_FN KealStr* keal_retain(KealStr* s) {
    if (s != NULL) {
        s->rc++;
    }
    return s;
}

KEAL_FN void keal_release(KealStr* s) {
    if (s == NULL) {
        return;
    }
    s->rc--;
    if (s->rc > 0) {
        return;
    }
    if (!s->static_bytes) {
        free((void*)s->bytes);
    }
    free(s);
}

/* ---- strings ---------------------------------------------------------- */

KEAL_FN KealStr* keal_str_owning(char* bytes, int64_t len) {
    KealStr* s = (KealStr*)keal_alloc(sizeof(KealStr));
    s->rc = 1;
    s->len = len;
    s->bytes = bytes;
    s->static_bytes = false;
    return s;
}

/* A literal. Its bytes live in the binary, so only the header is allocated,
 * and the count starts at one for the program's own reference. */
KEAL_FN KealStr* keal_str_static(const char* bytes, int64_t len) {
    KealStr* s = (KealStr*)keal_alloc(sizeof(KealStr));
    s->rc = 1;
    s->len = len;
    s->bytes = bytes;
    s->static_bytes = true;
    return s;
}

KEAL_FN KealStr* keal_str_empty(void) {
    return keal_str_static("", 0);
}

KEAL_FN KealStr* keal_concat(KealStr* a, KealStr* b) {
    int64_t len = a->len + b->len;
    char* bytes = (char*)keal_alloc((size_t)len + 1);
    memcpy(bytes, a->bytes, (size_t)a->len);
    memcpy(bytes + a->len, b->bytes, (size_t)b->len);
    bytes[len] = '\0';
    return keal_str_owning(bytes, len);
}

/* Byte order, which for UTF-8 is also code-point order. */
KEAL_FN int keal_str_cmp(KealStr* a, KealStr* b) {
    int64_t n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->bytes, b->bytes, (size_t)n);
    if (c != 0) {
        return c;
    }
    if (a->len == b->len) {
        return 0;
    }
    return a->len < b->len ? -1 : 1;
}

KEAL_FN KealStr* keal_str_from_bytes(const char* bytes, int64_t len) {
    char* copy = (char*)keal_alloc((size_t)len + 1);
    memcpy(copy, bytes, (size_t)len);
    copy[len] = '\0';
    return keal_str_owning(copy, len);
}

KEAL_FN KealStr* keal_str_from_int(int64_t n) {
    char buf[32];
    int len = snprintf(buf, sizeof buf, "%" PRId64, n);
    return keal_str_from_bytes(buf, len);
}

/* Matches how the other two engines render a float: a whole number keeps one
 * decimal place, so `1.0` does not print as `1`. */
KEAL_FN KealStr* keal_str_from_float(double d) {
    char buf[512];
    int len;
    if (d == (double)(int64_t)d && d < 1e15 && d > -1e15) {
        len = snprintf(buf, sizeof buf, "%.1f", d);
    } else {
        len = snprintf(buf, sizeof buf, "%g", d);
    }
    return keal_str_from_bytes(buf, len);
}

KEAL_FN KealStr* keal_str_from_bool(bool b) {
    return keal_str_static(b ? "true" : "false", b ? 4 : 5);
}

KEAL_FN void keal_print(KealStr* s, bool newline) {
    fwrite(s->bytes, 1, (size_t)s->len, stdout);
    if (newline) {
        fputc('\n', stdout);
    } else {
        fflush(stdout);
    }
}

/* ---- checked integer arithmetic --------------------------------------- */

/* The interpreter and the VM both refuse to wrap, so native code must not
 * either: a program that overflows should fail the same way whichever engine
 * runs it. */
KEAL_FN int64_t keal_add(int64_t a, int64_t b, int64_t line) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) {
        keal_panic("integer overflow", line);
    }
    return r;
}

KEAL_FN int64_t keal_sub(int64_t a, int64_t b, int64_t line) {
    int64_t r;
    if (__builtin_sub_overflow(a, b, &r)) {
        keal_panic("integer overflow", line);
    }
    return r;
}

KEAL_FN int64_t keal_mul(int64_t a, int64_t b, int64_t line) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) {
        keal_panic("integer overflow", line);
    }
    return r;
}

KEAL_FN int64_t keal_div(int64_t a, int64_t b, int64_t line) {
    if (b == 0) {
        keal_panic("division by zero", line);
    }
    if (a == INT64_MIN && b == -1) {
        keal_panic("integer overflow", line);
    }
    return a / b;
}

KEAL_FN int64_t keal_rem(int64_t a, int64_t b, int64_t line) {
    if (b == 0) {
        keal_panic("remainder by zero", line);
    }
    if (a == INT64_MIN && b == -1) {
        return 0;
    }
    return a % b;
}
