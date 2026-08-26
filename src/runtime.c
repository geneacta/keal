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
KEAL_FN KealStr* keal_str_retain(KealStr* s) {
    if (s != NULL) {
        s->rc++;
    }
    return s;
}

KEAL_FN void keal_str_release(KealStr* s) {
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

/* `.length` counts characters, not bytes, as the interpreters do. */
KEAL_FN int64_t keal_str_length(KealStr* s) {
    int64_t n = 0;
    for (int64_t i = 0; i < s->len; i++) {
        if (((unsigned char)s->bytes[i] & 0xC0) != 0x80) {
            n++;
        }
    }
    return n;
}

KEAL_FN void keal_print(KealStr* s, bool newline) {
    fwrite(s->bytes, 1, (size_t)s->len, stdout);
    if (newline) {
        fputc('\n', stdout);
    } else {
        fflush(stdout);
    }
}

/* ---- lists ------------------------------------------------------------ */

/* An element is one word: an integer, a double, or a pointer. The static
 * type at every use site says which, so the runtime never needs to ask. */
typedef union KealWord {
    int64_t i;
    double d;
    void* p;
} KealWord;

/* The one place an element's type is not statically known is inside the
 * list's own release, where the elements must be let go without any caller
 * present. The releaser is fixed at construction; NULL marks elements with
 * nothing to release. */
typedef struct KealList {
    int64_t rc;
    int64_t len;
    int64_t cap;
    KealWord* data;
    void (*release_elem)(void*);
} KealList;

KEAL_FN KealList* keal_list_new(void (*release_elem)(void*)) {
    KealList* l = (KealList*)keal_alloc(sizeof(KealList));
    l->rc = 1;
    l->len = 0;
    l->cap = 0;
    l->data = NULL;
    l->release_elem = release_elem;
    return l;
}

KEAL_FN KealList* keal_list_retain(KealList* l) {
    if (l != NULL) {
        l->rc++;
    }
    return l;
}

KEAL_FN void keal_list_release(KealList* l) {
    if (l == NULL) {
        return;
    }
    l->rc--;
    if (l->rc > 0) {
        return;
    }
    if (l->release_elem != NULL) {
        for (int64_t i = 0; i < l->len; i++) {
            l->release_elem(l->data[i].p);
        }
    }
    free(l->data);
    free(l);
}

KEAL_FN void keal_list_push(KealList* l, KealWord w) {
    if (l->len == l->cap) {
        l->cap = l->cap < 4 ? 4 : l->cap * 2;
        KealWord* grown = (KealWord*)keal_alloc((size_t)l->cap * sizeof(KealWord));
        memcpy(grown, l->data, (size_t)l->len * sizeof(KealWord));
        free(l->data);
        l->data = grown;
    }
    l->data[l->len++] = w;
}

/* Negative indices count from the end, as everywhere in the language. */
KEAL_FN int64_t keal_list_index(KealList* l, int64_t i, int64_t line) {
    int64_t at = i < 0 ? i + l->len : i;
    if (at < 0 || at >= l->len) {
        char msg[96];
        snprintf(msg, sizeof msg,
                 "index %" PRId64 " is out of bounds for a list of %" PRId64 " element(s)", i,
                 l->len);
        keal_panic(msg, line);
    }
    return at;
}

KEAL_FN KealWord keal_list_get(KealList* l, int64_t i, int64_t line) {
    return l->data[keal_list_index(l, i, line)];
}

/* The old element is handed back to the caller, which knows its type and
 * releases it — the runtime only stores. */
KEAL_FN KealWord keal_list_set(KealList* l, int64_t i, KealWord w, int64_t line) {
    int64_t at = keal_list_index(l, i, line);
    KealWord old = l->data[at];
    l->data[at] = w;
    return old;
}

/* A shallow copy for `for`, so the loop walks what the list held when it
 * started, whatever the body does to it — the interpreters' rule. Elements
 * are not retained: the copy is consumed by the loop before anything could
 * release the original's references out from under it, because the original
 * itself is still alive across the loop. */
KEAL_FN KealList* keal_list_snapshot(KealList* l) {
    KealList* c = keal_list_new(NULL);
    c->len = l->len;
    c->cap = l->len;
    c->data = (KealWord*)keal_alloc((size_t)(l->len < 1 ? 1 : l->len) * sizeof(KealWord));
    memcpy(c->data, l->data, (size_t)l->len * sizeof(KealWord));
    return c;
}

/* ---- building strings ------------------------------------------------- */

/* A growable buffer, used by the rendering functions the compiler generates
 * for each class. Concatenating with `keal_concat` would allocate once per
 * field; this allocates once per object. */
typedef struct KealBuf {
    char* data;
    int64_t len;
    int64_t cap;
} KealBuf;

KEAL_FN void keal_buf_init(KealBuf* b) {
    b->cap = 32;
    b->len = 0;
    b->data = (char*)keal_alloc((size_t)b->cap);
}

KEAL_FN void keal_buf_reserve(KealBuf* b, int64_t extra) {
    if (b->len + extra <= b->cap) {
        return;
    }
    while (b->cap < b->len + extra) {
        b->cap *= 2;
    }
    char* grown = (char*)keal_alloc((size_t)b->cap);
    memcpy(grown, b->data, (size_t)b->len);
    free(b->data);
    b->data = grown;
}

KEAL_FN void keal_buf_bytes(KealBuf* b, const char* s, int64_t n) {
    keal_buf_reserve(b, n);
    memcpy(b->data + b->len, s, (size_t)n);
    b->len += n;
}

KEAL_FN void keal_buf_lit(KealBuf* b, const char* s) {
    keal_buf_bytes(b, s, (int64_t)strlen(s));
}

/* Takes a reference and consumes it, which is what the generated rendering
 * code wants: it makes a string, appends it, and is done with it. */
KEAL_FN void keal_buf_str(KealBuf* b, KealStr* s) {
    keal_buf_bytes(b, s->bytes, s->len);
    keal_str_release(s);
}

KEAL_FN KealStr* keal_buf_finish(KealBuf* b) {
    keal_buf_reserve(b, 1);
    b->data[b->len] = '\0';
    return keal_str_owning(b->data, b->len);
}

/* How a string appears *inside* a rendered value: quoted and escaped, so
 * that `["a", "b"]` reads differently from `[a, b]`. Matches what the
 * interpreters print. */
KEAL_FN KealStr* keal_str_repr(KealStr* s) {
    KealBuf b;
    keal_buf_init(&b);
    keal_buf_lit(&b, "\"");
    for (int64_t i = 0; i < s->len; i++) {
        char c = s->bytes[i];
        switch (c) {
            case '"': keal_buf_lit(&b, "\\\""); break;
            case '\\': keal_buf_lit(&b, "\\\\"); break;
            case '\n': keal_buf_lit(&b, "\\n"); break;
            case '\t': keal_buf_lit(&b, "\\t"); break;
            case '\r': keal_buf_lit(&b, "\\r"); break;
            default: keal_buf_bytes(&b, &c, 1); break;
        }
    }
    keal_buf_lit(&b, "\"");
    keal_str_release(s);
    return keal_buf_finish(&b);
}

/* Comparing two strings either of which may be absent. Two absent strings
 * are equal; an absent one equals nothing else. */
KEAL_FN bool keal_opt_str_eq(KealStr* a, KealStr* b) {
    if (a == NULL || b == NULL) {
        return a == b;
    }
    return keal_str_cmp(a, b) == 0;
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
