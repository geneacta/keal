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

/* This file wants POSIX, and has to say so before the first include.
 *
 * `keal build` compiles with `-std=c11`, which defines `__STRICT_ANSI__`;
 * glibc then withdraws the declarations that are not in ISO C. It does not
 * withdraw all of them evenly — `opendir`, `fork`, `pipe` and `poll` stayed,
 * while `clock_gettime`, `CLOCK_REALTIME` and `localtime_r` vanished from
 * <time.h>, which is why this showed up as three names and not as a wall.
 * Apple's headers declare all of them regardless, so a program that compiled
 * here failed on the most ordinary machine there is.
 *
 * Not defined on Apple, where the same macro goes the other way and HIDES
 * the BSD extensions its headers assume. */
#if !defined(_WIN32) && !defined(__APPLE__)
#define _POSIX_C_SOURCE 200809L
#endif

#include <inttypes.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <time.h>

/* What the file-system primitives need, on both sides of the platform line.
 * Unconditional: a program reads a directory whether or not it has actors,
 * and `windows.h` guards itself against the copy the actor block includes.
 *
 * `WIN32_LEAN_AND_MEAN` leaves out the sockets, OLE, RPC, shell and
 * cryptography headers that `windows.h` otherwise drags in. Nothing here
 * touches any of them: the whole Windows surface this runtime uses is
 * `CreateThread`, `WaitForSingleObject`, `CloseHandle`, `GetSystemInfo`,
 * the `FindFirstFileW` family, `CreateFileW`, `CreateProcessW`, the
 * attribute and directory calls, and the two character conversions — all
 * of it kernel32, none of it excluded.
 *
 * Measured on Windows 10 with MinGW-W64 UCRT GCC 16.1: 505 headers and
 * 115,160 preprocessed lines become 278 and 79,865, and a translation unit
 * that costs 1,842 ms costs 1,408 — 23.6% off every file the backend emits.
 * An empty `main` cost more there than an 8,639-line program, because the
 * program is not what is being read. */
#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#else
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <sys/types.h>
#include <unistd.h>
#endif

/* Windows opens stdout in text mode, and text mode turns every `\n` this
 * runtime writes into `\r\n`. The three engines must print the same bytes —
 * that is the invariant the whole test suite rests on — so the native one
 * asks for the bytes it actually wrote. A constructor rather than a call in
 * `main`, so that not one line of generated C changes.
 *
 * `__attribute__((constructor))` is the same GCC/Clang ground the overflow
 * builtins below already stand on. */
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
__attribute__((constructor)) static void keal_stdio_is_bytes(void) {
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
}
#endif

/* A generated program uses only the part of this runtime it needs, so the
 * rest must not draw warnings from a caller building with -Wall. */
#if defined(__GNUC__) || defined(__clang__)
#define KEAL_FN __attribute__((unused)) static
#else
#define KEAL_FN static
#endif

/* ---- what weak references change --------------------------------------- */

/* A `weak` field does not keep its target alive. So an object needs a
 * second count — how many weak references name it — and the backend
 * defines KEAL_WEAK above this file exactly when the program declares a
 * weak field; without it, objects carry one count as they always have.
 *
 * When the strong count reaches zero the object runs its `deinit` and
 * releases its fields, as ever. What changes is the last step: it frees
 * itself only if no weak reference remains. Otherwise it stays as a
 * husk — a header with `rc == 0`, which *is* the "dead" test a weak read
 * makes — until the last weak reference goes and frees it. That is why a
 * weak read is always a safe read: the memory it inspects cannot have
 * been returned while it still names it. */

/* ---- what actors change ----------------------------------------------- */

/* An actor program runs its actors on real OS threads (docs/threads.md);
 * the backend defines KEAL_ACTORS above this file exactly when the program
 * touches the actor types. Two things follow, and only then:
 *
 * * **Counts go atomic.** Addresses (`ActorRef`, `Outbox`), the strings
 *   inside messages (immutable, so a copy shares them), and immutable
 *   globals are values two threads can see at once, and a plain count
 *   cannot say who frees last. The macros below are that one switch: a
 *   program without actors keeps plain counts and pays nothing.
 * * **One lock, one condition.** Every mailbox push, outbox post and
 *   drain happens under `keal_actor_mu`; actor threads sleep on
 *   `keal_actor_cv` until a message lands or the system stops. Message
 *   values themselves need no lock — a message is deep-copied before the
 *   push, and the mutex hand-off orders the copy before the read.
 */
#ifdef KEAL_ACTORS
#include <pthread.h>
#include <stdatomic.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <unistd.h>
#endif
typedef _Atomic int64_t keal_rc_t;
#define KEAL_RC_BUMP(rc) atomic_fetch_add_explicit(&(rc), 1, memory_order_relaxed)
#define KEAL_RC_DROP(rc) (atomic_fetch_sub_explicit(&(rc), 1, memory_order_acq_rel) > 1)

static pthread_mutex_t keal_actor_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t keal_actor_cv = PTHREAD_COND_INITIALIZER;

KEAL_FN void keal_actor_lock(void) { pthread_mutex_lock(&keal_actor_mu); }
KEAL_FN void keal_actor_unlock(void) { pthread_mutex_unlock(&keal_actor_mu); }
KEAL_FN void keal_actor_signal(void) { pthread_cond_broadcast(&keal_actor_cv); }
KEAL_FN void keal_actor_wait(void) { pthread_cond_wait(&keal_actor_cv, &keal_actor_mu); }

/* What one `run` shares with its actor threads: how many handlers are in
 * flight, the stop flag, and the first panic — carried back so it is
 * rethrown on the caller's thread, where `try { sys.run() }` can catch
 * it. All fields are guarded by `keal_actor_mu`. */
typedef struct KealRunState {
    int64_t workers;
    int stop;
    int panicked;
    int64_t panic_line;
    char panic_msg[1024];
    /* One flag per actor: whether a worker is inside its handler right now.
     * An actor handles its messages one at a time and in order, and that is
     * the whole of the actor model's promise — so two workers must never be
     * in one actor at once, however many actors and however few workers. */
    int8_t* busy;
    int64_t actors;
    /* Where the next worker starts scanning. Without it every worker would
     * begin at actor 0 and the ones at the end of the list would starve. */
    int64_t next;
} KealRunState;

/* How many threads run the actors.
 *
 * Not one per actor. A program with ten thousand actors would ask the
 * operating system for ten thousand threads — eighty megabytes of stacks
 * before a single message is handled, and a scheduler with nothing useful
 * to do. Actors are a way of writing a program, not a way of asking for
 * threads, so the count comes from the machine and the work is shared.
 *
 * `KEAL_ACTOR_WORKERS` overrides it, which is how a test pins the number
 * and how somebody measuring can walk it up and down. */
KEAL_FN int64_t keal_actor_worker_count(int64_t actors) {
    if (actors <= 0) {
        return 0;
    }
    int64_t want = 0;
    const char* env = getenv("KEAL_ACTOR_WORKERS");
    if (env != NULL && env[0] != '\0') {
        want = (int64_t)strtoll(env, NULL, 10);
    }
    if (want <= 0) {
#ifdef _WIN32
        SYSTEM_INFO si;
        GetSystemInfo(&si);
        want = (int64_t)si.dwNumberOfProcessors;
#else
        long n = sysconf(_SC_NPROCESSORS_ONLN);
        want = n > 0 ? (int64_t)n : 1;
#endif
    }
    if (want < 1) {
        want = 1;
    }
    /* More workers than actors is waste: an actor is never run by two at
     * once, so the extra threads would only ever scan and sleep. */
    return want < actors ? want : actors;
}
#else
typedef int64_t keal_rc_t;
#define KEAL_RC_BUMP(rc) ((rc)++)
#define KEAL_RC_DROP(rc) (--(rc) > 0)
#endif

/* ---- the cycle audit ----------------------------------------------------
 *
 * `keal build --audit` defines KEAL_AUDIT, and then every class counts its
 * live objects by name. What is still counted when the program ends is what
 * nothing could free — which, reference counting being what it is, is a
 * cycle. The interpreters answer the same question when KEAL_AUDIT is set in
 * the environment, and print the same report, because a program's answer
 * must not depend on which engine asked.
 *
 * Off, none of this exists and no object pays for it. */
#ifdef KEAL_AUDIT
typedef struct KealAuditRow {
    const char* name;
    int64_t live;
    /* How many of the live ones a top-level binding can reach, filled in
     * by the mark phase further down before the report is printed. */
    int64_t held;
} KealAuditRow;

static KealAuditRow keal_audit_rows[256];
static int keal_audit_used = 0;

#ifdef KEAL_ACTORS
/* Actors count from several threads at once, so the rows go under the lock
 * the scheduler already owns. */
KEAL_FN void keal_actor_lock(void);
KEAL_FN void keal_actor_unlock(void);
#define KEAL_AUDIT_LOCK() keal_actor_lock()
#define KEAL_AUDIT_UNLOCK() keal_actor_unlock()
#else
#define KEAL_AUDIT_LOCK() ((void)0)
#define KEAL_AUDIT_UNLOCK() ((void)0)
#endif

static KealAuditRow* keal_audit_row(const char* name) {
    for (int i = 0; i < keal_audit_used; i++) {
        if (strcmp(keal_audit_rows[i].name, name) == 0) { return &keal_audit_rows[i]; }
    }
    if (keal_audit_used == (int)(sizeof(keal_audit_rows) / sizeof(keal_audit_rows[0]))) {
        return NULL;
    }
    keal_audit_rows[keal_audit_used].name = name;
    keal_audit_rows[keal_audit_used].live = 0;
    keal_audit_rows[keal_audit_used].held = 0;
    return &keal_audit_rows[keal_audit_used++];
}

KEAL_FN void keal_audit_born(const char* name) {
    KEAL_AUDIT_LOCK();
    KealAuditRow* r = keal_audit_row(name);
    if (r != NULL) { r->live++; }
    KEAL_AUDIT_UNLOCK();
}

/* The mark phase's counter: this object is one a top-level binding can
 * reach, so it lived to the end because the program said so. */
KEAL_FN void keal_audit_hold(const char* name) {
    KEAL_AUDIT_LOCK();
    KealAuditRow* r = keal_audit_row(name);
    if (r != NULL) { r->held++; }
    KEAL_AUDIT_UNLOCK();
}

KEAL_FN void keal_audit_died(const char* name) {
    KEAL_AUDIT_LOCK();
    KealAuditRow* r = keal_audit_row(name);
    if (r != NULL) { r->live--; }
    KEAL_AUDIT_UNLOCK();
}

/* The same words, in the same order, as the interpreters print. */
KEAL_FN void keal_audit_report(void) {
    int order[256];
    int n = 0;
    int64_t total = 0;
    for (int i = 0; i < keal_audit_used; i++) {
        if (keal_audit_rows[i].live > 0) {
            order[n++] = i;
            total += keal_audit_rows[i].live;
        }
    }
    if (n == 0) {
        fprintf(stderr, "audit: nothing outlived the program\n");
        return;
    }
    for (int i = 1; i < n; i++) {
        int key = order[i];
        int j = i - 1;
        while (j >= 0 && strcmp(keal_audit_rows[order[j]].name, keal_audit_rows[key].name) > 0) {
            order[j + 1] = order[j];
            j--;
        }
        order[j + 1] = key;
    }
    fprintf(stderr, "audit: %" PRId64 " object(s) outlived the program\n", total);
    for (int i = 0; i < n; i++) {
        fprintf(stderr, "  %" PRId64 " %s\n", keal_audit_rows[order[i]].live,
                keal_audit_rows[order[i]].name);
    }
    /* The verdict the counts alone could not give: what the mark phase
     * reached lived to the end because a top-level binding said so, and
     * what it did not reach outlived its last reference. */
    int64_t lost = 0;
    int64_t kept = 0;
    for (int i = 0; i < n; i++) {
        KealAuditRow* r = &keal_audit_rows[order[i]];
        int64_t held = r->held > r->live ? r->live : r->held;
        lost += r->live - held;
        kept += held;
    }
    if (lost == 0) {
        fprintf(stderr, "  = note: every one of them is held by a top-level binding, which lives to the end of a program by design — none is a cycle\n");
        return;
    }
    fprintf(stderr, "  %" PRId64 " of them are reachable from no top-level binding, so they outlived their last reference — a cycle:\n", lost);
    for (int i = 0; i < n; i++) {
        KealAuditRow* r = &keal_audit_rows[order[i]];
        int64_t held = r->held > r->live ? r->live : r->held;
        if (r->live - held > 0) {
            fprintf(stderr, "    %" PRId64 " %s\n", r->live - held, r->name);
        }
    }
    if (kept > 0) {
        fprintf(stderr, "  the rest are held by a top-level binding, which lives to the end of a program by design\n");
    }
    fprintf(stderr, "  = note: `weak` on the back edge breaks a cycle — see docs/memory.md §5\n");
}
#endif

/* Every heap object begins with its count. A string is that count, a length,
 * and the bytes; `static_bytes` marks a literal, whose bytes are not ours to
 * free. */
typedef struct KealStr {
    keal_rc_t rc;
    int64_t len;
    const char* bytes;
    bool static_bytes;
} KealStr;

/* ---- catchable panics ------------------------------------------------ */

/* `try` blocks count themselves in and out; a panic under an active one
 * records its message and unwinds by poisoned returns — every helper that
 * can panic returns a harmless value right after, and the generated code
 * checks `keal_unwinding` before acting on any result. The backend only
 * emits those checks when the program contains a `try` at all, so programs
 * without one pay nothing and `keal_panic` still ends the process. */
/* Per-thread, so each future actor thread panics, catches and unwinds
 * on its own (docs/threads.md). Single-threaded programs see no change. */
static _Thread_local int64_t keal_try_depth = 0;
static _Thread_local bool keal_unwinding = false;
static _Thread_local char keal_unwind_msg[1024];
/* Whether a `throw` gave the unwind a value of its own. A built-in
 * failure gives none: its message is its value, exactly as `runtime::err`
 * builds a `String` for the interpreters, so `catch (e: String)` catches
 * an overflow on all three engines. */
static _Thread_local bool keal_unwind_has_value = false;
/* The line too, so a panic carried across threads — an actor's, rethrown
 * on the thread that called `run` — still names its source line. */
static _Thread_local int64_t keal_unwind_line = 0;

KEAL_FN void keal_panic(const char* what, int64_t line) {
    if (keal_try_depth > 0) {
        /* A panic while already unwinding keeps the first message. */
        if (!keal_unwinding) {
            keal_unwinding = true;
            keal_unwind_has_value = false;
            snprintf(keal_unwind_msg, sizeof keal_unwind_msg, "%s", what);
            keal_unwind_line = line;
        }
        return;
    }
    fprintf(stderr, "runtime error: %s\n", what);
    if (line > 0) {
        fprintf(stderr, "  at line %" PRId64 "\n", line);
    }
    exit(1);
}

/* Failures no `catch` can make right end the process even under a `try`. */
KEAL_FN void keal_fatal(const char* what) {
    fprintf(stderr, "runtime error: %s\n", what);
    exit(1);
}

KEAL_FN void* keal_alloc(size_t n) {
    void* p = malloc(n);
    if (p == NULL) {
        keal_fatal("out of memory");
    }
    return p;
}

/* ---- reference counting ---------------------------------------------- */

/* Returns its argument so that a retain can be written inline, which is what
 * lets the generated code read as expressions rather than as statements. */
KEAL_FN KealStr* keal_str_retain(KealStr* s) {
    if (s != NULL) {
        KEAL_RC_BUMP(s->rc);
    }
    return s;
}

KEAL_FN void keal_str_release(KealStr* s) {
    if (s == NULL) {
        return;
    }
    if (KEAL_RC_DROP(s->rc)) {
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

/* Matches how the other two engines render a float: a whole number keeps
 * one decimal place, so `1.0` does not print as `1`, and everything else
 * prints the SHORTEST decimal that reads back as exactly the same value —
 * the same answer Rust's formatter gives, found here by widening the
 * precision until the round-trip closes. */
KEAL_FN KealStr* keal_str_from_float(double d) {
    /* 5e-324 written out is "0." and 324 digits; the largest double is 309
     * digits before the point. 768 leaves room for both and the sign. */
    char buf[768];
    int len;
    /* `NaN`, as the two interpreters spell it. C's `%g` writes `nan`, and
     * the loop below could never have corrected it: it accepts a precision
     * when `strtod` reads the text back equal to `d`, and no NaN is equal
     * to itself. So every NaN fell through seventeen rounds of that and
     * landed on the C spelling — a three-engine disagreement on output,
     * which is the one thing the three are not allowed to have.
     * `inf` and `-inf` already agree. */
    if (d != d) {
        return keal_str_static("NaN", 3);
    }
    /* The range test first, and the cast second.
     *
     * These were the other way round, so every program that printed a float
     * outside int64_t's range converted one into it — undefined in C, and
     * caught by `-fsanitize=float-cast-overflow` on `1e300`. The range guard
     * behind it made the answer right anyway, which is why it survived: it
     * was undefined behaviour whose result was never used. `&&` sequences
     * left to right, so the cast now happens only where it is defined. */
    if (d > -1e15 && d < 1e15 && d == (double)(int64_t)d) {
        len = snprintf(buf, sizeof buf, "%.1f", d);
        return keal_str_from_bytes(buf, len);
    }
    /* The shortest digits that read back as this value, written out in
     * full — never in scientific notation, because the two interpreters
     * never write one.
     *
     * `%g` switches to an exponent as soon as one leaves [-4, precision),
     * so `1e15` printed as `1e+15` natively and `1000000000000000` on the
     * other two engines, and `1e300` printed as 1e+300 against three
     * hundred zeros. Everything outside [1e-4, 1e15) disagreed. The
     * precision loop could not correct it: `1e+15` reads back exactly, so
     * it was accepted on the first round.
     *
     * `%g` still finds the precision — it is the cheapest way to ask how
     * many significant digits round-trip — and then `%e` says where the
     * point sits, and `%f` writes it there. */
    for (int prec = 1; prec <= 17; prec++) {
        len = snprintf(buf, sizeof buf, "%.*g", prec, d);
        if (strtod(buf, NULL) != d) {
            continue;
        }
        if (strchr(buf, 'e') == NULL && strchr(buf, 'n') == NULL) {
            return keal_str_from_bytes(buf, len);
        }
        /* `inf`, which both other engines also write as `inf`. */
        if (strchr(buf, 'n') != NULL) {
            return keal_str_from_bytes(buf, len);
        }
        /* The digits themselves, laid out by hand.
         *
         * `%.*f` on the value would print what the double exactly IS —
         * 1e300 expands to 1000...0525047602552044..., three hundred digits
         * of binary truth. The other two engines print the shortest digits
         * that read back as this value, `1`, followed by three hundred
         * zeros. So the digits come from `%e`, which gives exactly those,
         * and the point goes where the exponent says. */
        char sci[64];
        snprintf(sci, sizeof sci, "%.*e", prec - 1, d);
        const char* p = sci;
        char sign = '\0';
        if (*p == '-') {
            sign = '-';
            p++;
        }
        char digits[24];
        int nd = 0;
        for (; *p != '\0' && *p != 'e' && nd < (int)sizeof digits - 1; p++) {
            if (*p != '.') {
                digits[nd++] = *p;
            }
        }
        digits[nd] = '\0';
        int exp10 = *p == 'e' ? atoi(p + 1) : 0;

        char* w = buf;
        if (sign != '\0') {
            *w++ = sign;
        }
        if (exp10 >= nd - 1) {
            /* Every digit is before the point, then zeros to reach it. */
            memcpy(w, digits, (size_t)nd);
            w += nd;
            for (int i = 0; i < exp10 - (nd - 1); i++) {
                *w++ = '0';
            }
        } else if (exp10 >= 0) {
            memcpy(w, digits, (size_t)exp10 + 1);
            w += exp10 + 1;
            *w++ = '.';
            memcpy(w, digits + exp10 + 1, (size_t)(nd - exp10 - 1));
            w += nd - exp10 - 1;
        } else {
            *w++ = '0';
            *w++ = '.';
            for (int i = 0; i < -exp10 - 1; i++) {
                *w++ = '0';
            }
            memcpy(w, digits, (size_t)nd);
            w += nd;
        }
        return keal_str_from_bytes(buf, (int)(w - buf));
    }
    len = snprintf(buf, sizeof buf, "%.17g", d);
    return keal_str_from_bytes(buf, len);
}

KEAL_FN KealStr* keal_str_from_bool(bool b) {
    return keal_str_static(b ? "true" : "false", b ? 4 : 5);
}

/* A comparison's outcome, as the word it is written with. The ordinal is
 * the whole value — 0, 1, 2 — the way `true` and `false` are the whole
 * value of a Bool. */
KEAL_FN KealStr* keal_str_from_comp(int64_t c) {
    if (c == 0) { return keal_str_static("less", 4); }
    if (c == 1) { return keal_str_static("equal", 5); }
    return keal_str_static("greater", 7);
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

/* ---- string methods ---------------------------------------------------- */

/* Byte offset of the n-th character; `n` past the end returns `len`. */
KEAL_FN int64_t keal_str_char_byte(KealStr* s, int64_t n) {
    int64_t seen = 0;
    for (int64_t i = 0; i < s->len; i++) {
        if (((unsigned char)s->bytes[i] & 0xC0) != 0x80) {
            if (seen == n) {
                return i;
            }
            seen++;
        }
    }
    return s->len;
}

KEAL_FN KealStr* keal_str_substring(KealStr* s, int64_t a, int64_t b, int64_t line) {
    int64_t len = keal_str_length(s);
    int64_t ca = a < 0 ? 0 : a;
    int64_t cb = b < 0 ? 0 : b;
    if (ca > cb || cb > len) {
        char msg[128];
        snprintf(msg, sizeof msg,
                 "substring(%" PRId64 ", %" PRId64 ") is out of range for a string of length %" PRId64,
                 ca, cb, len);
        keal_panic(msg, line);
        return NULL;
    }
    int64_t from = keal_str_char_byte(s, ca);
    int64_t to = keal_str_char_byte(s, cb);
    return keal_str_from_bytes(s->bytes + from, to - from);
}

KEAL_FN KealStr* keal_str_take(KealStr* s, int64_t n) {
    int64_t len = keal_str_length(s);
    int64_t k = n < 0 ? 0 : (n > len ? len : n);
    return keal_str_from_bytes(s->bytes, keal_str_char_byte(s, k));
}

KEAL_FN KealStr* keal_str_drop(KealStr* s, int64_t n) {
    int64_t len = keal_str_length(s);
    int64_t k = n < 0 ? 0 : (n > len ? len : n);
    int64_t from = keal_str_char_byte(s, k);
    return keal_str_from_bytes(s->bytes + from, s->len - from);
}

/* Byte position of `needle` in `s`, or -1. An empty needle is found at 0,
 * matching what the interpreters (and Rust's `find`) say. */
KEAL_FN int64_t keal_str_find_bytes(KealStr* s, KealStr* needle) {
    if (needle->len == 0) {
        return 0;
    }
    if (needle->len > s->len) {
        return -1;
    }
    for (int64_t i = 0; i + needle->len <= s->len; i++) {
        if (memcmp(s->bytes + i, needle->bytes, (size_t)needle->len) == 0) {
            return i;
        }
    }
    return -1;
}

KEAL_FN bool keal_str_contains(KealStr* s, KealStr* needle) {
    return keal_str_find_bytes(s, needle) >= 0;
}

KEAL_FN bool keal_str_starts_with(KealStr* s, KealStr* prefix) {
    return prefix->len <= s->len
        && memcmp(s->bytes, prefix->bytes, (size_t)prefix->len) == 0;
}

KEAL_FN bool keal_str_ends_with(KealStr* s, KealStr* suffix) {
    return suffix->len <= s->len
        && memcmp(s->bytes + (s->len - suffix->len), suffix->bytes,
                  (size_t)suffix->len) == 0;
}

/* The index in characters, not bytes, as the interpreters report it. */
KEAL_FN int64_t keal_str_index_of(KealStr* s, KealStr* needle) {
    int64_t byte = keal_str_find_bytes(s, needle);
    if (byte < 0) {
        return -1;
    }
    int64_t chars = 0;
    for (int64_t i = 0; i < byte; i++) {
        if (((unsigned char)s->bytes[i] & 0xC0) != 0x80) {
            chars++;
        }
    }
    return chars;
}

KEAL_FN KealStr* keal_str_replace(KealStr* s, KealStr* old, KealStr* newer, int64_t line) {
    if (old->len == 0) {
        keal_panic("`replace` needs a non-empty search string", line);
        return NULL;
    }
    /* KealBuf is declared later in this file, so the bytes are collected by
     * hand into a local buffer. */
    int64_t cap = s->len + 16;
    int64_t len = 0;
    char* out = (char*)keal_alloc((size_t)cap);
    int64_t i = 0;
    while (i < s->len) {
        if (i + old->len <= s->len
            && memcmp(s->bytes + i, old->bytes, (size_t)old->len) == 0) {
            while (len + newer->len > cap) {
                cap *= 2;
                char* grown = (char*)keal_alloc((size_t)cap);
                memcpy(grown, out, (size_t)len);
                free(out);
                out = grown;
            }
            memcpy(out + len, newer->bytes, (size_t)newer->len);
            len += newer->len;
            i += old->len;
        } else {
            while (len + 1 > cap) {
                cap *= 2;
                char* grown = (char*)keal_alloc((size_t)cap);
                memcpy(grown, out, (size_t)len);
                free(out);
                out = grown;
            }
            out[len++] = s->bytes[i++];
        }
    }
    KealStr* r = keal_str_from_bytes(out, len);
    free(out);
    return r;
}

KEAL_FN KealStr* keal_str_repeat(KealStr* s, int64_t n, int64_t line) {
    if (n < 0) {
        char msg[96];
        snprintf(msg, sizeof msg, "`repeat` needs a non-negative count, got %" PRId64, n);
        keal_panic(msg, line);
        return NULL;
    }
    int64_t total = s->len * n;
    char* out = (char*)keal_alloc((size_t)(total < 1 ? 1 : total));
    for (int64_t i = 0; i < n; i++) {
        memcpy(out + i * s->len, s->bytes, (size_t)s->len);
    }
    KealStr* r = keal_str_from_bytes(out, total);
    free(out);
    return r;
}

/* The first code point, or -1 for an empty string. */
KEAL_FN int64_t keal_str_code(KealStr* s) {
    if (s->len == 0) {
        return -1;
    }
    unsigned char b0 = (unsigned char)s->bytes[0];
    if (b0 < 0x80) {
        return b0;
    }
    if ((b0 & 0xE0) == 0xC0 && s->len >= 2) {
        return ((int64_t)(b0 & 0x1F) << 6) | (s->bytes[1] & 0x3F);
    }
    if ((b0 & 0xF0) == 0xE0 && s->len >= 3) {
        return ((int64_t)(b0 & 0x0F) << 12) | ((int64_t)(s->bytes[1] & 0x3F) << 6)
            | (s->bytes[2] & 0x3F);
    }
    if ((b0 & 0xF8) == 0xF0 && s->len >= 4) {
        return ((int64_t)(b0 & 0x07) << 18) | ((int64_t)(s->bytes[1] & 0x3F) << 12)
            | ((int64_t)(s->bytes[2] & 0x3F) << 6) | (s->bytes[3] & 0x3F);
    }
    return b0;
}

KEAL_FN KealStr* keal_int_to_char(int64_t n, int64_t line) {
    bool valid = n >= 0 && n <= 0x10FFFF && !(n >= 0xD800 && n <= 0xDFFF);
    if (!valid) {
        char msg[96];
        snprintf(msg, sizeof msg, "%" PRId64 " is not a valid character code", n);
        keal_panic(msg, line);
        return NULL;
    }
    char buf[4];
    int len;
    if (n < 0x80) {
        buf[0] = (char)n;
        len = 1;
    } else if (n < 0x800) {
        buf[0] = (char)(0xC0 | (n >> 6));
        buf[1] = (char)(0x80 | (n & 0x3F));
        len = 2;
    } else if (n < 0x10000) {
        buf[0] = (char)(0xE0 | (n >> 12));
        buf[1] = (char)(0x80 | ((n >> 6) & 0x3F));
        buf[2] = (char)(0x80 | (n & 0x3F));
        len = 3;
    } else {
        buf[0] = (char)(0xF0 | (n >> 18));
        buf[1] = (char)(0x80 | ((n >> 12) & 0x3F));
        buf[2] = (char)(0x80 | ((n >> 6) & 0x3F));
        buf[3] = (char)(0x80 | (n & 0x3F));
        len = 4;
    }
    return keal_str_from_bytes(buf, len);
}

/* `Float.toInt` truncates toward zero and saturates at the edges — the
 * semantics of Rust's `as i64`, which is what the interpreters run. */
KEAL_FN int64_t keal_f2i(double d) {
    if (d != d) {
        return 0;
    }
    if (d >= 9223372036854775807.0) {
        return INT64_MAX;
    }
    if (d <= -9223372036854775808.0) {
        return INT64_MIN;
    }
    return (int64_t)d;
}

KEAL_FN int64_t keal_int_abs(int64_t n) {
    if (n < 0) {
        return (int64_t)(0 - (uint64_t)n);
    }
    return n;
}

KEAL_FN int64_t keal_int_pow(int64_t n, int64_t e, int64_t line) {
    if (e < 0) {
        char msg[96];
        snprintf(msg, sizeof msg, "`Int.pow` needs a non-negative exponent, got %" PRId64, e);
        keal_panic(msg, line);
        return 0;
    }
    int64_t r = 1;
    for (int64_t i = 0; i < e; i++) {
        if (__builtin_mul_overflow(r, n, &r)) {
            char msg[96];
            snprintf(msg, sizeof msg, "integer overflow in %" PRId64 ".pow(%" PRId64 ")", n, e);
            keal_panic(msg, line);
            return 0;
        }
    }
    return r;
}

/* The integer d-th root: the largest r >= 0 with r**d <= n — the inverse
 * of `**` on the whole numbers, shared by `^/`, `^/=` and `Int.root`. */
KEAL_FN int64_t keal_int_root(int64_t n, int64_t d, int64_t line) {
    if (d <= 0) {
        char msg[96];
        snprintf(msg, sizeof msg, "`root` needs a positive degree, got %" PRId64, d);
        keal_panic(msg, line);
        return 0;
    }
    if (n < 0) {
        keal_panic("cannot take the root of a negative number", line);
        return 0;
    }
    if (n == 0) {
        return 0;
    }
    int64_t r = (int64_t)floor(pow((double)n, 1.0 / (double)d));
    if (r < 1) {
        r = 1;
    }
    /* Exact fixup: r**d computed with overflow watched, so the float
     * estimate can neither over- nor under-shoot the answer. */
    for (;;) {
        int64_t c = r + 1;
        int64_t acc = 1;
        bool over = false;
        for (int64_t i = 0; i < d; i++) {
            if (__builtin_mul_overflow(acc, c, &acc) || acc > n) {
                over = true;
                break;
            }
        }
        if (over) {
            break;
        }
        r = c;
    }
    for (;;) {
        int64_t acc = 1;
        bool over = false;
        for (int64_t i = 0; i < d; i++) {
            if (__builtin_mul_overflow(acc, r, &acc) || acc > n) {
                over = true;
                break;
            }
        }
        if (!over) {
            break;
        }
        r -= 1;
    }
    return r;
}

/* The d-th root of a float: IEEE all the way — a negative base is NaN, not
 * a panic, exactly as `**` with a fractional exponent would be. */
KEAL_FN double keal_f_root(double x, double d) {
    return pow(x, 1.0 / d);
}

KEAL_FN int64_t keal_int_min(int64_t a, int64_t b) { return a < b ? a : b; }

KEAL_FN int64_t keal_int_max(int64_t a, int64_t b) { return a > b ? a : b; }

/* Adopts a NUL-terminated buffer from C: Keal owns it from here and will
 * free() it when the last reference goes. The other half of `own String`.
 * A NULL from C reads as the empty string. */
KEAL_FN KealStr* keal_str_adopt(char* bytes) {
    if (bytes == NULL) {
        return keal_str_empty();
    }
    return keal_str_owning(bytes, (int64_t)strlen(bytes));
}

/* `s[i]` and `s.get(i)`: one character, as a string. Negative counts from
 * the end. */
KEAL_FN KealStr* keal_str_get(KealStr* s, int64_t i, int64_t line) {
    int64_t len = keal_str_length(s);
    int64_t idx = i < 0 ? i + len : i;
    if (idx < 0 || idx >= len) {
        char msg[128];
        snprintf(msg, sizeof msg,
                 "index %" PRId64 " is out of bounds for a string of length %" PRId64, i, len);
        keal_panic(msg, line);
        return NULL;
    }
    int64_t from = keal_str_char_byte(s, idx);
    int64_t to = keal_str_char_byte(s, idx + 1);
    return keal_str_from_bytes(s->bytes + from, to - from);
}

/* Case mapping and trim are ASCII: the interpreters use Unicode tables the
 * runtime does not carry, so non-ASCII cased letters pass through unchanged.
 * ASCII covers what the language's own tooling needs. */
KEAL_FN KealStr* keal_str_to_lower(KealStr* s) {
    char* out = (char*)keal_alloc((size_t)(s->len < 1 ? 1 : s->len));
    for (int64_t i = 0; i < s->len; i++) {
        char c = s->bytes[i];
        out[i] = (c >= 'A' && c <= 'Z') ? (char)(c + 32) : c;
    }
    KealStr* r = keal_str_from_bytes(out, s->len);
    free(out);
    return r;
}

KEAL_FN KealStr* keal_str_to_upper(KealStr* s) {
    char* out = (char*)keal_alloc((size_t)(s->len < 1 ? 1 : s->len));
    for (int64_t i = 0; i < s->len; i++) {
        char c = s->bytes[i];
        out[i] = (c >= 'a' && c <= 'z') ? (char)(c - 32) : c;
    }
    KealStr* r = keal_str_from_bytes(out, s->len);
    free(out);
    return r;
}

KEAL_FN KealStr* keal_str_trim(KealStr* s) {
    int64_t a = 0;
    int64_t b = s->len;
    while (a < b && (s->bytes[a] == ' ' || s->bytes[a] == '\t' || s->bytes[a] == '\n' || s->bytes[a] == '\r')) {
        a++;
    }
    while (b > a && (s->bytes[b - 1] == ' ' || s->bytes[b - 1] == '\t' || s->bytes[b - 1] == '\n' || s->bytes[b - 1] == '\r')) {
        b--;
    }
    return keal_str_from_bytes(s->bytes + a, b - a);
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
    keal_rc_t rc;
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
        KEAL_RC_BUMP(l->rc);
    }
    return l;
}

KEAL_FN void keal_list_release(KealList* l) {
    if (l == NULL) {
        return;
    }
    if (KEAL_RC_DROP(l->rc)) {
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
        return 0;
    }
    return at;
}

KEAL_FN KealWord keal_list_get(KealList* l, int64_t i, int64_t line) {
    int64_t at = keal_list_index(l, i, line);
    if (keal_unwinding) {
        KealWord none = {0};
        return none;
    }
    return l->data[at];
}

/* The old element is handed back to the caller, which knows its type and
 * releases it — the runtime only stores. */
KEAL_FN KealWord keal_list_set(KealList* l, int64_t i, KealWord w, int64_t line) {
    int64_t at = keal_list_index(l, i, line);
    if (keal_unwinding) {
        KealWord none = {0};
        return none;
    }
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

/* Removal hands the element's reference to the caller, which knows the type
 * and owns it from here. */
KEAL_FN KealWord keal_list_remove_at(KealList* l, int64_t i, int64_t line) {
    int64_t idx = i < 0 ? i + l->len : i;
    if (idx < 0 || idx >= l->len) {
        char msg[96];
        snprintf(msg, sizeof msg,
                 "index %" PRId64 " is out of bounds for a list of %" PRId64, i, l->len);
        keal_panic(msg, line);
        KealWord none = {0};
        return none;
    }
    KealWord out = l->data[idx];
    memmove(l->data + idx, l->data + idx + 1,
            (size_t)(l->len - idx - 1) * sizeof(KealWord));
    l->len--;
    return out;
}

/* Takes ownership of the word, like push. */
KEAL_FN void keal_list_insert_at(KealList* l, int64_t i, KealWord w, int64_t line) {
    if (i < 0 || i > l->len) {
        char msg[96];
        snprintf(msg, sizeof msg,
                 "cannot insert at index %" PRId64 " in a list of %" PRId64, i, l->len);
        keal_panic(msg, line);
        return;
    }
    keal_list_push(l, w);
    memmove(l->data + i + 1, l->data + i, (size_t)(l->len - i - 1) * sizeof(KealWord));
    l->data[i] = w;
}

/* `xs.set(i, v)` — the method, not the indexing form. It counts a negative
 * index from the end like everything else, but its out-of-range message is
 * the method's own: the shorter one `removeAt` gives, not the "element(s)"
 * one `keal_list_index` gives for `xs[i]`. The interpreters draw the same
 * line, so the runtime does too. The displaced element is handed back to
 * the caller, which knows its type and releases it. */
KEAL_FN KealWord keal_list_set_at(KealList* l, int64_t i, KealWord w, int64_t line) {
    int64_t idx = i < 0 ? i + l->len : i;
    if (idx < 0 || idx >= l->len) {
        char msg[96];
        snprintf(msg, sizeof msg,
                 "index %" PRId64 " is out of bounds for a list of %" PRId64, i, l->len);
        keal_panic(msg, line);
        KealWord none = {0};
        return none;
    }
    KealWord old = l->data[idx];
    l->data[idx] = w;
    return old;
}

/* `xs.clear()` — every element let go, front to back, by the releaser fixed
 * at construction, exactly as the list's own death lets them go. The buffer
 * is kept: capacity is not something a program can observe. Nothing user
 * code wrote can run inside the loop, because a `deinit` is queued rather
 * than called from a release. */
KEAL_FN void keal_list_clear(KealList* l) {
    if (l->release_elem != NULL) {
        for (int64_t i = 0; i < l->len; i++) {
            l->release_elem(l->data[i].p);
        }
    }
    l->len = 0;
}

/* Every counted object leads with its count — the memory model's promise —
 * so one raw bump serves whatever a pointer element points at. */
KEAL_FN void keal_word_retain_raw(void* p) {
    if (p != NULL) {
        (*(int64_t*)p)++;
    }
}

/* `dst.addAll(src)`: dst takes a reference of its own to each element. */
KEAL_FN void keal_list_add_all(KealList* dst, KealList* src) {
    int64_t n = src->len;
    for (int64_t i = 0; i < n; i++) {
        KealWord w = src->data[i];
        if (dst->release_elem != NULL) {
            keal_word_retain_raw(w.p);
        }
        keal_list_push(dst, w);
    }
}

/* `take` and `drop` build new lists; each element carried over gets a
 * reference of its own, by the raw bump every counted object supports. */
KEAL_FN KealList* keal_list_take(KealList* l, int64_t n) {
    int64_t k = n < 0 ? 0 : (n > l->len ? l->len : n);
    KealList* out = keal_list_new(l->release_elem);
    for (int64_t i = 0; i < k; i++) {
        if (out->release_elem != NULL) {
            keal_word_retain_raw(l->data[i].p);
        }
        keal_list_push(out, l->data[i]);
    }
    return out;
}

/* `xs.slice(start, end)` — end exclusive, both clamped, and an empty list
 * where they cross. The interpreters clamp rather than panic, so this does
 * too: asking for more than there is answers with what there is. */
KEAL_FN KealList* keal_list_slice(KealList* l, int64_t start, int64_t end) {
    int64_t a = start < 0 ? 0 : (start > l->len ? l->len : start);
    int64_t b = end < 0 ? 0 : (end > l->len ? l->len : end);
    KealList* out = keal_list_new(l->release_elem);
    for (int64_t i = a; i < b; i++) {
        if (out->release_elem != NULL) {
            keal_word_retain_raw(l->data[i].p);
        }
        keal_list_push(out, l->data[i]);
    }
    return out;
}

KEAL_FN KealList* keal_list_drop(KealList* l, int64_t n) {
    int64_t k = n < 0 ? 0 : (n > l->len ? l->len : n);
    KealList* out = keal_list_new(l->release_elem);
    for (int64_t i = k; i < l->len; i++) {
        if (out->release_elem != NULL) {
            keal_word_retain_raw(l->data[i].p);
        }
        keal_list_push(out, l->data[i]);
    }
    return out;
}

/* `xs.reversed()` — a new list, back to front, the receiver untouched;
 * each element carried over gets a reference of its own, the same way
 * `take` and `drop` take theirs. */
KEAL_FN KealList* keal_list_reversed(KealList* l) {
    KealList* out = keal_list_new(l->release_elem);
    for (int64_t i = l->len - 1; i >= 0; i--) {
        if (out->release_elem != NULL) {
            keal_word_retain_raw(l->data[i].p);
        }
        keal_list_push(out, l->data[i]);
    }
    return out;
}

KEAL_FN int keal_cmp_i64(KealWord a, KealWord b) {
    return a.i < b.i ? -1 : (a.i > b.i ? 1 : 0);
}

KEAL_FN int keal_cmp_str(KealWord a, KealWord b) {
    return keal_str_cmp((KealStr*)a.p, (KealStr*)b.p);
}

/* A stable in-place insertion sort by the given comparator. */
KEAL_FN void keal_list_sort_words(KealList* items, int (*cmp)(KealWord, KealWord)) {
    for (int64_t i = 1; i < items->len; i++) {
        KealWord it = items->data[i];
        int64_t j = i - 1;
        while (j >= 0 && cmp(items->data[j], it) > 0) {
            items->data[j + 1] = items->data[j];
            j--;
        }
        items->data[j + 1] = it;
    }
}

KEAL_FN bool keal_key_eq_f64(KealWord a, KealWord b) {
    return a.d == b.d;
}

KEAL_FN bool keal_key_eq_opt_str(KealWord a, KealWord b) {
    KealStr* x = (KealStr*)a.p;
    KealStr* y = (KealStr*)b.p;
    if (x == NULL || y == NULL) {
        return x == y;
    }
    return keal_str_cmp(x, y) == 0;
}

KEAL_FN bool keal_list_contains(KealList* l, KealWord w, bool (*eq)(KealWord, KealWord)) {
    for (int64_t i = 0; i < l->len; i++) {
        if (eq(l->data[i], w)) {
            return true;
        }
    }
    return false;
}

/* `xs.indexOf(v)` — the first index whose element compares equal, and -1
 * when none does, which is the answer both interpreters give. The same
 * equality `contains` scans with, so the two never disagree. */
KEAL_FN int64_t keal_list_index_of(KealList* l, KealWord w, bool (*eq)(KealWord, KealWord)) {
    for (int64_t i = 0; i < l->len; i++) {
        if (eq(l->data[i], w)) {
            return i;
        }
    }
    return -1;
}

/* Structural equality, as the interpreters compare lists: same length,
 * elements equal pairwise. Null equals only null. */
KEAL_FN bool keal_list_eq(KealList* a, KealList* b, bool (*eq)(KealWord, KealWord)) {
    if (a == NULL || b == NULL || a == b) {
        return a == b;
    }
    if (a->len != b->len) {
        return false;
    }
    for (int64_t i = 0; i < a->len; i++) {
        if (!eq(a->data[i], b->data[i])) {
            return false;
        }
    }
    return true;
}

/* forward declaration: maps are declared below, their equality with them. */
typedef struct KealMap KealMap;
KEAL_FN int64_t keal_map_find(KealMap* m, KealWord key);

/* A stable insertion sort of `items` by the parallel Int keys — small,
 * predictable, and stable, which is the semantics the interpreters give. */
KEAL_FN void keal_list_sort_by_i64(KealList* items, KealList* keys) {
    for (int64_t i = 1; i < items->len; i++) {
        KealWord it = items->data[i];
        KealWord k = keys->data[i];
        int64_t j = i - 1;
        while (j >= 0 && keys->data[j].i > k.i) {
            items->data[j + 1] = items->data[j];
            keys->data[j + 1] = keys->data[j];
            j--;
        }
        items->data[j + 1] = it;
        keys->data[j + 1] = k;
    }
}

/* `xs.sum()` over Ints, checked: the language refuses to wrap, so a total
 * that will not fit fails here rather than quietly turning negative — and
 * with the interpreters' wording, so a `catch` reads the same message on
 * all three engines. */
KEAL_FN int64_t keal_list_sum_i64(KealList* l, int64_t line) {
    int64_t total = 0;
    for (int64_t i = 0; i < l->len; i++) {
        int64_t r;
        if (__builtin_add_overflow(total, l->data[i].i, &r)) {
            keal_panic("integer overflow in `sum`", line);
            return 0;
        }
        total = r;
    }
    return total;
}

/* And over Floats, which cannot overflow: a running total added front to
 * back, so the rounding is the interpreters' rounding, element for
 * element. */
KEAL_FN double keal_list_sum_f64(KealList* l) {
    double total = 0.0;
    for (int64_t i = 0; i < l->len; i++) {
        total += l->data[i].d;
    }
    return total;
}

/* ---- maps ------------------------------------------------------------- */

/* Entries in insertion order, which is the language's iteration order, with
 * keys and values interleaved. Lookup is a linear scan: correct first, and
 * honest about it — a hash table can replace the scan without changing any
 * caller.
 *
 * Keys compare by the comparator fixed at construction: one for word-sized
 * keys (Int, Bool, and Float by bit pattern, as the interpreters key them),
 * one for strings by content. */
typedef struct KealMap {
    keal_rc_t rc;
    int64_t len;
    int64_t cap;
    KealWord* data; /* [key, value, key, value, ...] */
    bool (*key_eq)(KealWord, KealWord);
    void (*release_key)(void*);
    void (*release_val)(void*);
    /* A key type with finitely many values indexes this directly.
     *
     * `domain` is how many values the key type has — 2 for a Bool, 3 for a
     * Comp, one per variant for an enum — and `slot[ordinal]` is where that
     * key's entry sits in `data`, or -1. Lookup stops being a scan and
     * becomes a read, without changing anything a program can observe:
     * `data` still holds the entries in the order they were first set,
     * which is what `keys()` promises.
     *
     * NULL for every other key type, and then nothing below behaves
     * differently from how it did. */
    int64_t* slot;
    int64_t domain;
    /* For every other key type: an open-addressed table from hash to the
     * position of the entry in `data`, holding position + 1 so that zero
     * means empty.
     *
     * The entries themselves are untouched — they stay in the order they
     * were first set, which `keys()` promises — and this only says where to
     * look. The interpreters have had a hash map here since they were
     * written; the native backend walked the entries, so the three engines
     * agreed on every answer and disagreed on what it cost.
     *
     * NULL when `key_hash` is NULL, and then the scan below still works. */
    int64_t* bucket;
    int64_t nbuckets;
    uint64_t (*key_hash)(KealWord);
} KealMap;

/* A word key — an Int, a Bool, an enum's ordinal, a Comp, or a Float's bit
 * pattern. Splitmix64's finaliser: cheap, and it moves every input bit. */
KEAL_FN uint64_t keal_hash_word(KealWord k) {
    uint64_t x = (uint64_t)k.i;
    x ^= x >> 30;
    x *= 0xbf58476d1ce4e5b9ULL;
    x ^= x >> 27;
    x *= 0x94d049bb133111ebULL;
    x ^= x >> 31;
    return x;
}

/* A string key, hashed over its bytes so that two equal strings agree
 * whatever their addresses. FNV-1a. A NULL string is its own hash, which is
 * what a nullable key needs. */
KEAL_FN uint64_t keal_hash_str(KealWord k) {
    KealStr* s = (KealStr*)k.p;
    if (s == NULL) {
        return 0xcbf29ce484222325ULL;
    }
    uint64_t h = 0xcbf29ce484222325ULL;
    for (int64_t i = 0; i < s->len; i++) {
        h ^= (uint64_t)(unsigned char)s->bytes[i];
        h *= 0x100000001b3ULL;
    }
    return h;
}

KEAL_FN bool keal_key_eq_word(KealWord a, KealWord b) {
    return a.i == b.i;
}

KEAL_FN bool keal_key_eq_str(KealWord a, KealWord b) {
    return keal_str_cmp((KealStr*)a.p, (KealStr*)b.p) == 0;
}

KEAL_FN KealMap* keal_map_new(bool (*key_eq)(KealWord, KealWord),
                              uint64_t (*key_hash)(KealWord),
                              void (*release_key)(void*), void (*release_val)(void*)) {
    KealMap* m = (KealMap*)keal_alloc(sizeof(KealMap));
    m->rc = 1;
    m->len = 0;
    m->cap = 0;
    m->data = NULL;
    m->key_eq = key_eq;
    m->release_key = release_key;
    m->release_val = release_val;
    m->slot = NULL;
    m->domain = 0;
    m->bucket = NULL;
    m->nbuckets = 0;
    m->key_hash = key_hash;
    return m;
}

/* The same map, with an ordinal index because the key type has finitely
 * many values. `domain` is how many: 2 for a Bool, 3 for a Comp, one per
 * variant for an enum. Nothing a program can observe differs — the entries
 * still sit in the order they were first set — only the cost of finding
 * one, which stops depending on how many there are. */
KEAL_FN KealMap* keal_map_new_closed(bool (*key_eq)(KealWord, KealWord),
                                     uint64_t (*key_hash)(KealWord),
                                     void (*release_key)(void*),
                                     void (*release_val)(void*), int64_t domain) {
    KealMap* m = keal_map_new(key_eq, key_hash, release_key, release_val);
    if (domain <= 0) {
        /* Not a closed key after all — a plain map, so that a caller which
         * passes another map's domain along does the right thing either
         * way. */
        return m;
    }
    m->domain = domain;
    m->slot = (int64_t*)keal_alloc((size_t)domain * sizeof(int64_t));
    for (int64_t i = 0; i < domain; i++) {
        m->slot[i] = -1;
    }
    return m;
}

KEAL_FN KealMap* keal_map_retain(KealMap* m) {
    if (m != NULL) {
        KEAL_RC_BUMP(m->rc);
    }
    return m;
}

KEAL_FN void keal_map_release(KealMap* m) {
    if (m == NULL) {
        return;
    }
    if (KEAL_RC_DROP(m->rc)) {
        return;
    }
    for (int64_t i = 0; i < m->len; i++) {
        if (m->release_key != NULL) {
            m->release_key(m->data[2 * i].p);
        }
        if (m->release_val != NULL) {
            m->release_val(m->data[2 * i + 1].p);
        }
    }
    free(m->data);
    free(m->slot);
    free(m->bucket);
    free(m);
}

/* The entry's index, or -1. */
KEAL_FN int64_t keal_map_find(KealMap* m, KealWord key) {
    if (m->slot != NULL) {
        if (key.i < 0 || key.i >= m->domain) {
            return -1;
        }
        return m->slot[key.i];
    }
    if (m->bucket != NULL) {
        uint64_t mask = (uint64_t)m->nbuckets - 1;
        uint64_t i = m->key_hash(key) & mask;
        while (m->bucket[i] != 0) {
            int64_t at = m->bucket[i] - 1;
            /* The hash says where to look; equality says whether it is the
             * one. Two keys may land in the same bucket and still differ. */
            if (m->key_eq(m->data[2 * at], key)) {
                return at;
            }
            i = (i + 1) & mask;
        }
        return -1;
    }
    for (int64_t i = 0; i < m->len; i++) {
        if (m->key_eq(m->data[2 * i], key)) {
            return i;
        }
    }
    return -1;
}

/* Puts one entry into the bucket table. The table always has a free slot
 * because it is grown before it fills. */
KEAL_FN void keal_map_bucket_put(KealMap* m, KealWord key, int64_t at) {
    uint64_t mask = (uint64_t)m->nbuckets - 1;
    uint64_t i = m->key_hash(key) & mask;
    while (m->bucket[i] != 0) {
        i = (i + 1) & mask;
    }
    m->bucket[i] = at + 1;
}

/* Grows the bucket table to hold `want` entries at half load, and refills it
 * from the entries. Also the way it is built the first time. */
KEAL_FN void keal_map_rehash(KealMap* m, int64_t want) {
    int64_t n = 8;
    while (n < want * 2) {
        n *= 2;
    }
    free(m->bucket);
    m->nbuckets = n;
    m->bucket = (int64_t*)keal_alloc((size_t)n * sizeof(int64_t));
    for (int64_t i = 0; i < n; i++) {
        m->bucket[i] = 0;
    }
    for (int64_t i = 0; i < m->len; i++) {
        keal_map_bucket_put(m, m->data[2 * i], i);
    }
}

/* Rebuilds whichever index this map has, after the entries move. Only a
 * removal moves them, and only by shifting the tail down one — which
 * changes the position of every entry after it, so both kinds of index have
 * to be rebuilt rather than patched. */
KEAL_FN void keal_map_reindex(KealMap* m) {
    if (m->slot != NULL) {
        for (int64_t i = 0; i < m->domain; i++) {
            m->slot[i] = -1;
        }
        for (int64_t i = 0; i < m->len; i++) {
            m->slot[m->data[2 * i].i] = i;
        }
        return;
    }
    if (m->bucket != NULL) {
        keal_map_rehash(m, m->len);
    }
}

/* Takes ownership of both words; on a replaced entry the displaced key and
 * value are released here, since the map is the only one who knows them. */
KEAL_FN void keal_map_set(KealMap* m, KealWord key, KealWord value) {
    int64_t at = keal_map_find(m, key);
    if (at >= 0) {
        if (m->release_key != NULL) {
            m->release_key(m->data[2 * at].p);
        }
        if (m->release_val != NULL) {
            m->release_val(m->data[2 * at + 1].p);
        }
        m->data[2 * at] = key;
        m->data[2 * at + 1] = value;
        return;
    }
    if (m->len == m->cap) {
        m->cap = m->cap < 4 ? 4 : m->cap * 2;
        KealWord* grown = (KealWord*)keal_alloc((size_t)m->cap * 2 * sizeof(KealWord));
        memcpy(grown, m->data, (size_t)m->len * 2 * sizeof(KealWord));
        free(m->data);
        m->data = grown;
    }
    m->data[2 * m->len] = key;
    m->data[2 * m->len + 1] = value;
    if (m->slot != NULL) {
        m->slot[key.i] = m->len;
    } else if (m->key_hash != NULL) {
        /* Half load, so a probe stays short. */
        if (m->bucket == NULL || (m->len + 1) * 2 > m->nbuckets) {
            keal_map_rehash(m, m->len + 1);
        }
        keal_map_bucket_put(m, key, m->len);
    }
    m->len++;
}

/* Removes an entry, keeping the order of the ones around it: a map here
 * remembers the order its keys were first set, and `keys()` promises it, so
 * a removal cannot be the usual swap-with-the-last. */
KEAL_FN void keal_map_remove(KealMap* m, KealWord key) {
    int64_t at = keal_map_find(m, key);
    if (at < 0) {
        return;
    }
    if (m->release_key != NULL) {
        m->release_key(m->data[2 * at].p);
    }
    if (m->release_val != NULL) {
        m->release_val(m->data[2 * at + 1].p);
    }
    memmove(m->data + 2 * at, m->data + 2 * (at + 1),
            (size_t)(m->len - at - 1) * 2 * sizeof(KealWord));
    m->len--;
    keal_map_reindex(m);
}

/* Structural map equality, as the interpreters compare maps: same size, and
 * every entry of `a` present in `b` with an equal value — insertion order
 * does not matter. Keys compare by the map's own comparator. */
/* `m.clear()` — every key and every value let go by the releasers fixed at
 * construction, the way the map's own death lets them go. The buffer stays:
 * capacity is not something a program can observe. */
KEAL_FN void keal_map_clear(KealMap* m) {
    for (int64_t i = 0; i < m->len; i++) {
        if (m->release_key != NULL) {
            m->release_key(m->data[2 * i].p);
        }
        if (m->release_val != NULL) {
            m->release_val(m->data[2 * i + 1].p);
        }
    }
    m->len = 0;
    /* The entries are gone, so the index that says where they were has to
     * go with them. It did not, and the result was not a slow map but a
     * stuck one: `clear` left every bucket holding the position of an entry
     * that no longer counted, the next fill probed past all of them, and
     * the fill after that met a table with no free slot at all and looped
     * for ever. Two clears were needed to reach it, which is why a corpus
     * that fills a map and reads it never went near.
     *
     * `remove` already did this, because removing shifts the tail and moves
     * every position after it. Emptying the map moves them all. */
    keal_map_reindex(m);
}

KEAL_FN bool keal_map_eq(KealMap* a, KealMap* b, bool (*val_eq)(KealWord, KealWord)) {
    if (a == NULL || b == NULL || a == b) {
        return a == b;
    }
    if (a->len != b->len) {
        return false;
    }
    for (int64_t i = 0; i < a->len; i++) {
        int64_t at = keal_map_find(b, a->data[2 * i]);
        if (at < 0) {
            return false;
        }
        if (!val_eq(a->data[2 * i + 1], b->data[2 * at + 1])) {
            return false;
        }
    }
    return true;
}

/* A snapshot of the keys, for iteration; borrows, like the list snapshot. */
KEAL_FN KealList* keal_map_keys_snapshot(KealMap* m) {
    KealList* l = keal_list_new(NULL);
    for (int64_t i = 0; i < m->len; i++) {
        keal_list_push(l, m->data[2 * i]);
    }
    return l;
}

/* ---- strings as lists -------------------------------------------------- */

KEAL_FN void rel_keal_str_thunk(void* p);

/* One string per character, as `chars()` and `for (c in s)` see them. */
KEAL_FN KealList* keal_str_chars(KealStr* s) {
    KealList* l = keal_list_new(rel_keal_str_thunk);
    int64_t i = 0;
    while (i < s->len) {
        int64_t j = i + 1;
        while (j < s->len && ((unsigned char)s->bytes[j] & 0xC0) == 0x80) {
            j++;
        }
        keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(s->bytes + i, j - i) });
        i = j;
    }
    return l;
}

/* An empty separator splits into characters; otherwise the separator's
 * occurrences bound the parts, empty parts included — Rust's `split`. */
KEAL_FN KealList* keal_str_split(KealStr* s, KealStr* sep) {
    if (sep->len == 0) {
        return keal_str_chars(s);
    }
    KealList* l = keal_list_new(rel_keal_str_thunk);
    int64_t start = 0;
    int64_t i = 0;
    while (i + sep->len <= s->len) {
        if (memcmp(s->bytes + i, sep->bytes, (size_t)sep->len) == 0) {
            keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(s->bytes + start, i - start) });
            i += sep->len;
            start = i;
        } else {
            i++;
        }
    }
    keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(s->bytes + start, s->len - start) });
    return l;
}

/* `join` over strings; the default separator is the caller's business. */
KEAL_FN KealStr* keal_list_join_str(KealList* l, KealStr* sep) {
    int64_t total = 0;
    for (int64_t i = 0; i < l->len; i++) {
        total += ((KealStr*)l->data[i].p)->len;
    }
    if (l->len > 1) {
        total += sep->len * (l->len - 1);
    }
    char* out = (char*)keal_alloc((size_t)(total < 1 ? 1 : total));
    int64_t at = 0;
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) {
            memcpy(out + at, sep->bytes, (size_t)sep->len);
            at += sep->len;
        }
        KealStr* part = (KealStr*)l->data[i].p;
        memcpy(out + at, part->bytes, (size_t)part->len);
        at += part->len;
    }
    KealStr* r = keal_str_from_bytes(out, total);
    free(out);
    return r;
}

/* ---- closures --------------------------------------------------------- */

/* A function value: the count, the code, and how to drop the environment.
 * Each lambda's captures follow this header in a struct of its own; the call
 * site casts `fn` to the signature the static type promises. */
typedef void (*KealCode)(void);

typedef struct KealClosure {
    keal_rc_t rc;
    KealCode fn;
    void (*drop)(struct KealClosure* self);
    /* A fresh closure whose captured values are deep copies — the spawn
     * semantics: an actor's state is its own. NULL when the captures do
     * not copy, which the checker keeps unreachable through `spawn`. */
    struct KealClosure* (*copy)(struct KealClosure* self);
} KealClosure;

KEAL_FN KealClosure* keal_fn_copy_captures(KealClosure* c) {
    if (c == NULL || c->copy == NULL) {
        keal_panic("this handler's captures do not copy", 0);
        return NULL;
    }
    return c->copy(c);
}

KEAL_FN KealClosure* keal_fn_retain(KealClosure* c) {
    if (c != NULL) {
        KEAL_RC_BUMP(c->rc);
    }
    return c;
}

KEAL_FN void keal_fn_release(KealClosure* c) {
    if (c == NULL) {
        return;
    }
    if (KEAL_RC_DROP(c->rc)) {
        return;
    }
    c->drop(c);
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

/* ---- Any --------------------------------------------------------------- */

/* A value whose type is not known statically: a tag and a payload, two
 * words in all, exactly as `keal layout` prices it. The tag is a pointer
 * to the type's info — what `is` compares, what `typeOf` names, and what
 * tells retain/release whether the payload owns a reference. NULL tag
 * means the `Any` holds null. Only same-layout values enter an `Any`
 * natively (the backend refuses the rest by name), so what the tag says
 * is always what the payload is. */
typedef struct KealTypeInfo {
    const char* name;
    void (*retain)(void* p);
    void (*release)(void* p);
    KealStr* (*show)(KealWord w);
    bool (*eq)(KealWord a, KealWord b);
} KealTypeInfo;

typedef struct KealAny {
    const KealTypeInfo* ti;
    KealWord w;
} KealAny;

/* A list or map element slot is one word, and an `Any` is two — so inside
 * a container an `Any` lives behind one counted pointer. */
typedef struct KealAnyBox {
    keal_rc_t rc;
    KealAny a;
} KealAnyBox;

KEAL_FN KealAny keal_any_retain(KealAny a) {
    if (a.ti != NULL && a.ti->retain != NULL) {
        a.ti->retain(a.w.p);
    }
    return a;
}

KEAL_FN void keal_any_release(KealAny a) {
    if (a.ti != NULL && a.ti->release != NULL) {
        a.ti->release(a.w.p);
    }
}

KEAL_FN KealAny keal_any_null(void) {
    KealAny a;
    a.ti = NULL;
    a.w.i = 0;
    return a;
}

/* A pointer payload whose value may be null: a null pointer in, null out. */
KEAL_FN KealAny keal_any_of_ptr(const KealTypeInfo* ti, void* p) {
    if (p == NULL) {
        return keal_any_null();
    }
    KealAny a;
    a.ti = ti;
    a.w.p = p;
    return a;
}

KEAL_FN void* keal_any_box(KealAny a) {
    KealAnyBox* b = (KealAnyBox*)keal_alloc(sizeof(KealAnyBox));
    b->rc = 1;
    b->a = a;
    return b;
}

KEAL_FN void keal_any_box_release(void* p) {
    KealAnyBox* b = (KealAnyBox*)p;
    if (b == NULL) {
        return;
    }
    if (KEAL_RC_DROP(b->rc)) {
        return;
    }
    keal_any_release(b->a);
    free(b);
}

/* Equality the way the interpreters' values_equal answers it: same tag,
 * then by structure for data and by identity for instances and boxes'
 * contents follow the same rule. Two nulls are equal. */
KEAL_FN bool keal_any_eq(KealAny a, KealAny b) {
    if (a.ti != b.ti) {
        return false;
    }
    if (a.ti == NULL) {
        return true;
    }
    return a.ti->eq(a.w, b.w);
}

KEAL_FN bool keal_any_box_eq(KealWord a, KealWord b) {
    return keal_any_eq(((KealAnyBox*)a.p)->a, ((KealAnyBox*)b.p)->a);
}

/* Rendering inside a container: strings quoted, like the interpreters. */
KEAL_FN KealStr* keal_any_repr(KealAny a) {
    if (a.ti == NULL) {
        return keal_str_static("null", 4);
    }
    return a.ti->show(a.w);
}

/* What `println` and `${...}` produce: a bare string stays bare. */
KEAL_FN KealStr* keal_any_display(KealAny a);

KEAL_FN KealStr* keal_any_type_name(KealAny a) {
    if (a.ti == NULL) {
        return keal_str_static("Null", 4);
    }
    return keal_str_static(a.ti->name, (int64_t)strlen(a.ti->name));
}

static void keal_any_ti_retain_str(void* p) { keal_str_retain((KealStr*)p); }
static void keal_any_ti_release_str(void* p) { keal_str_release((KealStr*)p); }
static KealStr* keal_any_ti_show_int(KealWord w) { return keal_str_from_int(w.i); }
static KealStr* keal_any_ti_show_float(KealWord w) { return keal_str_from_float(w.d); }
static KealStr* keal_any_ti_show_bool(KealWord w) { return keal_str_from_bool((bool)w.i); }
static KealStr* keal_any_ti_show_str(KealWord w) {
    return keal_str_repr(keal_str_retain((KealStr*)w.p));
}
static bool keal_any_ti_eq_word(KealWord a, KealWord b) { return a.i == b.i; }
static bool keal_any_ti_eq_float(KealWord a, KealWord b) { return a.d == b.d; }
static bool keal_any_ti_eq_str(KealWord a, KealWord b) {
    return keal_str_cmp((KealStr*)a.p, (KealStr*)b.p) == 0;
}
/* Instances compare by identity inside an `Any`, exactly as the
 * interpreters compare dynamic values; the structural `==` a record
 * enjoys is the checker's rewrite to `equals`, which `Any` never takes. */
KEAL_FN bool keal_any_ptr_eq(KealWord a, KealWord b) { return a.p == b.p; }

static const KealTypeInfo keal_ti_int = {
    "Int", NULL, NULL, keal_any_ti_show_int, keal_any_ti_eq_word
};
static const KealTypeInfo keal_ti_float = {
    "Float", NULL, NULL, keal_any_ti_show_float, keal_any_ti_eq_float
};
static const KealTypeInfo keal_ti_bool = {
    "Bool", NULL, NULL, keal_any_ti_show_bool, keal_any_ti_eq_word
};
static const KealTypeInfo keal_ti_str = {
    "String", keal_any_ti_retain_str, keal_any_ti_release_str,
    keal_any_ti_show_str, keal_any_ti_eq_str
};

/* The one list that fits in an `Any` is `List<Any>` — same layout seen
 * from every side, which is what keeps `is List` honest natively. */
static void keal_any_ti_retain_list(void* p) { keal_list_retain((KealList*)p); }
static void keal_any_ti_release_list(void* p) { keal_list_release((KealList*)p); }
static KealStr* keal_any_ti_show_list(KealWord w) {
    KealList* l = (KealList*)w.p;
    KealBuf b;
    keal_buf_init(&b);
    keal_buf_lit(&b, "[");
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) {
            keal_buf_lit(&b, ", ");
        }
        keal_buf_str(&b, keal_any_repr(((KealAnyBox*)l->data[i].p)->a));
    }
    keal_buf_lit(&b, "]");
    return keal_buf_finish(&b);
}
static bool keal_any_ti_eq_list(KealWord a, KealWord b) {
    return keal_list_eq((KealList*)a.p, (KealList*)b.p, keal_any_box_eq);
}
static const KealTypeInfo keal_ti_list = {
    "List", keal_any_ti_retain_list, keal_any_ti_release_list,
    keal_any_ti_show_list, keal_any_ti_eq_list
};

KEAL_FN KealStr* keal_any_display(KealAny a) {
    if (a.ti == NULL) {
        return keal_str_static("null", 4);
    }
    if (a.ti == &keal_ti_str) {
        return keal_str_retain((KealStr*)a.w.p);
    }
    return a.ti->show(a.w);
}

/* Comparing two strings either of which may be absent. Two absent strings
 * are equal; an absent one equals nothing else. */
KEAL_FN bool keal_opt_str_eq(KealStr* a, KealStr* b) {
    if (a == NULL || b == NULL) {
        return a == b;
    }
    return keal_str_cmp(a, b) == 0;
}

/* ---- nullable values -------------------------------------------------- */

/* `Int?` and `Float?`: a tag beside the value, exactly as `keal layout`
 * prices them. `Bool?` instead uses a spare pattern of its byte — 0, 1, and
 * 2 for null — which is the layout table's promise kept. */
typedef struct KealOptI64 {
    bool has;
    int64_t v;
} KealOptI64;

typedef struct KealOptF64 {
    bool has;
    double v;
} KealOptF64;

/* Tagged optionals flowing into an `Any`: present becomes the value's own
 * tag, absent becomes the null `Any` — one more case the tag had free. */
KEAL_FN KealAny keal_any_of_opt_i64(KealOptI64 o) {
    KealAny a;
    if (!o.has) {
        return keal_any_null();
    }
    a.ti = &keal_ti_int;
    a.w.i = o.v;
    return a;
}

KEAL_FN KealAny keal_any_of_opt_f64(KealOptF64 o) {
    KealAny a;
    if (!o.has) {
        return keal_any_null();
    }
    a.ti = &keal_ti_float;
    a.w.d = o.v;
    return a;
}

KEAL_FN KealAny keal_any_of_opt_bool(int8_t o) {
    KealAny a;
    if (o == 2) {
        return keal_any_null();
    }
    a.ti = &keal_ti_bool;
    a.w.i = (int64_t)o;
    return a;
}

#ifdef KEAL_AUDIT
/* ---- the audit's mark phase ------------------------------------------- */

/* The rule the counts alone could not give, and the same rule both
 * interpreters apply — it has to be the same, or a program's answer would
 * depend on which engine asked. A top-level binding lives to the end of a
 * program by design, and so does everything it holds: all of that is
 * reachable from a root. Anything alive that no root reaches outlived its
 * last reference, which nothing but a cycle can do.
 *
 * The walk follows only the edges that hold an object up. A `weak` field
 * holds nothing up, so the generated walk skips one: were it followed, the
 * back edge of a cycle would report its own cycle as reachable, which is
 * exactly the answer this exists to stop giving.
 *
 * None of it costs a program anything: it is compiled only under
 * `keal build --audit`. */

typedef void (*KealAuditMark)(void*);

/* A shape is walked by pairing its release function — which the backend
 * already emits, and already hands to every list, map and closure — with
 * the mark that walks the same fields. Both come from one place in the
 * backend, so the two cannot drift apart, and no runtime structure had to
 * grow a field to carry the second one. */
typedef struct KealAuditPair {
    void* rel;
    KealAuditMark mark;
} KealAuditPair;

static KealAuditPair keal_audit_pairs[512];
static int keal_audit_pairs_used = 0;

KEAL_FN void keal_audit_pair(void* rel, KealAuditMark mark) {
    if (rel == NULL) { return; }
    for (int i = 0; i < keal_audit_pairs_used; i++) {
        if (keal_audit_pairs[i].rel == rel) { return; }
    }
    if (keal_audit_pairs_used == (int)(sizeof keal_audit_pairs / sizeof keal_audit_pairs[0])) {
        keal_fatal("audit: too many shapes to walk");
    }
    keal_audit_pairs[keal_audit_pairs_used].rel = rel;
    keal_audit_pairs[keal_audit_pairs_used].mark = mark;
    keal_audit_pairs_used++;
}

static KealAuditMark keal_audit_mark_for(void* rel) {
    for (int i = 0; i < keal_audit_pairs_used; i++) {
        if (keal_audit_pairs[i].rel == rel) { return keal_audit_pairs[i].mark; }
    }
    /* Silence here would under-count, and under-counting reports a cycle
     * that is not one. An audit that cannot walk a shape says so. */
    keal_fatal("audit: a shape the mark phase does not know how to walk");
    return NULL;
}

/* Every object is visited once, which is what makes a walk over the very
 * cycles it is looking for terminate. */
static void** keal_audit_seen = NULL;
static size_t keal_audit_seen_len = 0;
static size_t keal_audit_seen_cap = 0;

static bool keal_audit_first(void* p) {
    for (size_t i = 0; i < keal_audit_seen_len; i++) {
        if (keal_audit_seen[i] == p) { return false; }
    }
    if (keal_audit_seen_len == keal_audit_seen_cap) {
        size_t cap = keal_audit_seen_cap == 0 ? 64 : keal_audit_seen_cap * 2;
        void** grown = (void**)realloc(keal_audit_seen, cap * sizeof(void*));
        if (grown == NULL) { keal_fatal("out of memory"); }
        keal_audit_seen = grown;
        keal_audit_seen_cap = cap;
    }
    keal_audit_seen[keal_audit_seen_len++] = p;
    return true;
}

/* The mark phase's own scratch, freed once it has answered — an audit that
 * leaked would be the last thing to leak. */
KEAL_FN void keal_audit_done(void) {
    free(keal_audit_seen);
    keal_audit_seen = NULL;
    keal_audit_seen_len = 0;
    keal_audit_seen_cap = 0;
}

/* A string holds nothing, and neither does a scalar. */
KEAL_FN void keal_str_mark(void* p) { (void)p; }

KEAL_FN void keal_any_mark(KealAny a);

KEAL_FN void keal_list_mark(void* p) {
    KealList* l = (KealList*)p;
    if (l == NULL || !keal_audit_first(l)) { return; }
    /* No element releaser means the elements are scalars, which hold
     * nothing — the same test the release path makes. */
    if (l->release_elem == NULL) { return; }
    KealAuditMark m = keal_audit_mark_for((void*)l->release_elem);
    for (int64_t i = 0; i < l->len; i++) { m(l->data[i].p); }
}

KEAL_FN void keal_map_mark(void* p) {
    KealMap* m = (KealMap*)p;
    if (m == NULL || !keal_audit_first(m)) { return; }
    KealAuditMark mk = m->release_key == NULL ? NULL : keal_audit_mark_for((void*)m->release_key);
    KealAuditMark mv = m->release_val == NULL ? NULL : keal_audit_mark_for((void*)m->release_val);
    for (int64_t i = 0; i < m->len; i++) {
        if (mk != NULL) { mk(m->data[i * 2].p); }
        if (mv != NULL) { mv(m->data[i * 2 + 1].p); }
    }
}

/* A closure's captures live in a struct only its own code knows the shape
 * of, so its walk is paired with its `drop` — the one function the header
 * already carries that names that shape. */
KEAL_FN void keal_fn_mark(void* p) {
    KealClosure* c = (KealClosure*)p;
    if (c == NULL || !keal_audit_first(c)) { return; }
    keal_audit_mark_for((void*)c->drop)(c);
}

KEAL_FN void keal_any_mark(KealAny a) {
    if (a.ti == NULL || a.ti->release == NULL) { return; }
    keal_audit_mark_for((void*)a.ti->release)(a.w.p);
}

KEAL_FN void keal_any_box_mark(void* p) {
    KealAnyBox* b = (KealAnyBox*)p;
    if (b == NULL || !keal_audit_first(b)) { return; }
    keal_any_mark(b->a);
}

#endif

/* `String.toInt`: trimmed, an optional sign, digits, nothing else, and no
 * overflow — the grammar Rust's `i64::from_str` accepts, which is what the
 * interpreters run. */
KEAL_FN KealOptI64 keal_str_to_int(KealStr* s) {
    KealOptI64 none = { false, 0 };
    int64_t a = 0;
    int64_t b = s->len;
    while (a < b && (s->bytes[a] == ' ' || s->bytes[a] == '\t' || s->bytes[a] == '\n' || s->bytes[a] == '\r')) {
        a++;
    }
    while (b > a && (s->bytes[b - 1] == ' ' || s->bytes[b - 1] == '\t' || s->bytes[b - 1] == '\n' || s->bytes[b - 1] == '\r')) {
        b--;
    }
    if (a >= b) {
        return none;
    }
    bool neg = false;
    if (s->bytes[a] == '+' || s->bytes[a] == '-') {
        neg = s->bytes[a] == '-';
        a++;
    }
    if (a >= b) {
        return none;
    }
    int64_t value = 0;
    for (int64_t i = a; i < b; i++) {
        char c = s->bytes[i];
        if (c < '0' || c > '9') {
            return none;
        }
        if (__builtin_mul_overflow(value, 10, &value)) {
            return none;
        }
        int64_t d = neg ? -(c - '0') : (c - '0');
        if (__builtin_add_overflow(value, d, &value)) {
            return none;
        }
    }
    KealOptI64 some = { true, value };
    return some;
}

/* `String.toFloat`: validated against the grammar Rust's `f64::from_str`
 * accepts — no hex floats, at least one digit — then read by `strtod`. */
KEAL_FN KealOptF64 keal_str_to_float(KealStr* s) {
    KealOptF64 none = { false, 0.0 };
    int64_t a = 0;
    int64_t b = s->len;
    while (a < b && (s->bytes[a] == ' ' || s->bytes[a] == '\t' || s->bytes[a] == '\n' || s->bytes[a] == '\r')) {
        a++;
    }
    while (b > a && (s->bytes[b - 1] == ' ' || s->bytes[b - 1] == '\t' || s->bytes[b - 1] == '\n' || s->bytes[b - 1] == '\r')) {
        b--;
    }
    if (a >= b || b - a >= 512) {
        return none;
    }
    char buf[512];
    memcpy(buf, s->bytes + a, (size_t)(b - a));
    buf[b - a] = '\0';
    /* Validate: [+-]? (inf | infinity | nan | digits[.digits][e[+-]digits]
     * with a digit somewhere and no hex). */
    const char* p = buf;
    if (*p == '+' || *p == '-') {
        p++;
    }
    bool word = strcmp(p, "inf") == 0 || strcmp(p, "infinity") == 0 || strcmp(p, "nan") == 0
        || strcmp(p, "NaN") == 0 || strcmp(p, "Inf") == 0;
    if (!word) {
        bool digit = false;
        bool dot = false;
        bool exp = false;
        for (const char* q = p; *q; q++) {
            char c = *q;
            if (c >= '0' && c <= '9') {
                digit = true;
            } else if (c == '.' && !dot && !exp) {
                dot = true;
            } else if ((c == 'e' || c == 'E') && !exp && digit) {
                exp = true;
                if (q[1] == '+' || q[1] == '-') {
                    q++;
                }
                if (q[1] < '0' || q[1] > '9') {
                    return none;
                }
            } else {
                return none;
            }
        }
        if (!digit) {
            return none;
        }
    }
    char* end = NULL;
    double v = strtod(buf, &end);
    if (end == NULL || *end != '\0') {
        return none;
    }
    KealOptF64 some = { true, v };
    return some;
}

/* ---- cells ------------------------------------------------------------ */

/* A mutable variable some closure captures. The frame and every closure
 * share the one cell, so an assignment through any of them is seen by all —
 * the interpreters' semantics, kept rather than approximated. */
typedef struct KealCell {
    keal_rc_t rc;
    KealWord w;
    void (*release_inner)(void*);
} KealCell;

KEAL_FN KealCell* keal_cell_new(void (*release_inner)(void*)) {
    KealCell* c = (KealCell*)keal_alloc(sizeof(KealCell));
    c->rc = 1;
    c->w.i = 0;
    c->release_inner = release_inner;
    return c;
}

KEAL_FN KealCell* keal_cell_retain(KealCell* c) {
    if (c != NULL) {
        KEAL_RC_BUMP(c->rc);
    }
    return c;
}

#ifdef KEAL_AUDIT
/* A captured `var` lives in a shared cell, so the walk goes through one. */
KEAL_FN void keal_cell_mark(void* p) {
    KealCell* c = (KealCell*)p;
    if (c == NULL || !keal_audit_first(c)) { return; }
    if (c->release_inner == NULL) { return; }
    keal_audit_mark_for((void*)c->release_inner)(c->w.p);
}
#endif

KEAL_FN void keal_cell_release(KealCell* c) {
    if (c == NULL) {
        return;
    }
    if (KEAL_RC_DROP(c->rc)) {
        return;
    }
    if (c->release_inner != NULL) {
        c->release_inner(c->w.p);
    }
    free(c);
}

#ifdef KEAL_AUDIT
/* The shapes the runtime owns, paired once. What a program adds — one per
 * class, one per lambda, one per element type a container was made for —
 * the backend pairs beside the release it already emits, so a shape and
 * the walk over it are written in the same place and cannot drift. */
KEAL_FN void keal_audit_pair_runtime(void) {
    keal_audit_pair((void*)keal_any_box_release, keal_any_box_mark);
    keal_audit_pair((void*)keal_any_ti_release_str, keal_str_mark);
    keal_audit_pair((void*)keal_any_ti_release_list, keal_list_mark);
    keal_audit_pair((void*)keal_cell_release, keal_cell_mark);
    keal_audit_pair((void*)keal_str_release, keal_str_mark);
    keal_audit_pair((void*)keal_list_release, keal_list_mark);
    keal_audit_pair((void*)keal_map_release, keal_map_mark);
    keal_audit_pair((void*)keal_fn_release, keal_fn_mark);
}
#endif


/* ---- the host --------------------------------------------------------- */

/* Filled by main before anything runs; what `args()` returns. */
static int keal_argc = 0;
static char** keal_argv = NULL;

KEAL_FN void rel_keal_str_thunk(void* p) { keal_str_release((KealStr*)p); }

KEAL_FN KealList* keal_args(void) {
    KealList* l = keal_list_new(rel_keal_str_thunk);
    for (int i = 0; i < keal_argc; i++) {
        keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(keal_argv[i],
                                                              (int64_t)strlen(keal_argv[i])) });
    }
    return l;
}

#ifdef _WIN32
/* A Keal string is UTF-8, and the ANSI entry points read it as the active
 * code page. That is not merely lossy: it is SELF-CONSISTENT, because the
 * same wrong conversion happens on the way out again — a program that makes
 * a directory called `日本` and lists it back sees `日本`, while what is on
 * disk is `æ—¥æœ¬` and no other tool on the machine can open it. Worse, a
 * file some other program created cannot be seen at all: `isDir` answers
 * false about a directory that exists, and `listDir` returns question marks
 * where the code page has no room. Nothing written in Keal can detect any of
 * this, because every test that makes its own tree agrees with itself.
 *
 * So the file system uses the wide entry points, and these two convert at
 * the boundary. Free the result of `keal_widen_bytes` with `free`. */
static wchar_t* keal_widen_bytes(const char* utf8, int64_t len) {
    int n = MultiByteToWideChar(CP_UTF8, 0, utf8, (int)len, NULL, 0);
    if (n < 0) {
        return NULL;
    }
    wchar_t* w = (wchar_t*)malloc(((size_t)n + 1) * sizeof(wchar_t));
    if (w == NULL) {
        return NULL;
    }
    if (n > 0) {
        MultiByteToWideChar(CP_UTF8, 0, utf8, (int)len, w, n);
    }
    w[n] = L'\0';
    return w;
}

/* A path as the wide entry points want it. NULL on failure, which every
 * caller turns into the same absence a missing file gives. */
static wchar_t* keal_wpath(KealStr* path) {
    if (path->len < 0) {
        return NULL;
    }
    return keal_widen_bytes(path->bytes, path->len);
}

/* And back: a name the file system handed over, as the UTF-8 a Keal string
 * is made of. */
static KealStr* keal_narrow(const wchar_t* w) {
    int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    if (n <= 0) {
        return keal_str_from_bytes("", 0);
    }
    char* buf = (char*)malloc((size_t)n);
    if (buf == NULL) {
        return keal_str_from_bytes("", 0);
    }
    WideCharToMultiByte(CP_UTF8, 0, w, -1, buf, n, NULL, NULL);
    KealStr* out = keal_str_from_bytes(buf, (int64_t)(n - 1));
    free(buf);
    return out;
}
#endif

/* A file, opened by the name the program actually gave. On Windows that
 * means the wide entry point: `fopen` reads the name as the active code
 * page, so a file called `été` would be created under another name and a
 * file some other program wrote could not be found at all. */
static FILE* keal_open(KealStr* path, const char* cpath, bool writing) {
#ifdef _WIN32
    (void)cpath;
    wchar_t* wp = keal_wpath(path);
    if (wp == NULL) {
        return NULL;
    }
    FILE* f = _wfopen(wp, writing ? L"wb" : L"rb");
    free(wp);
    return f;
#else
    (void)path;
    return fopen(cpath, writing ? "wb" : "rb");
#endif
}

/* NULL on any failure: the caller's `?:` decides what absence means. */
KEAL_FN KealStr* keal_read_file(KealStr* path) {
    char cpath[4096];
    if (path->len >= (int64_t)sizeof cpath) {
        return NULL;
    }
    memcpy(cpath, path->bytes, (size_t)path->len);
    cpath[path->len] = '\0';
    FILE* f = keal_open(path, cpath, false);
    if (f == NULL) {
        return NULL;
    }
    if (fseek(f, 0, SEEK_END) != 0) {
        fclose(f);
        return NULL;
    }
    long size = ftell(f);
    if (size < 0) {
        fclose(f);
        return NULL;
    }
    rewind(f);
    char* bytes = (char*)keal_alloc((size_t)size + 1);
    size_t got = fread(bytes, 1, (size_t)size, f);
    fclose(f);
    if (got != (size_t)size) {
        free(bytes);
        return NULL;
    }
    bytes[size] = '\0';
    return keal_str_owning(bytes, (int64_t)size);
}

/* `s.reversed()` — by CHARACTER, not by byte. Reversing the bytes of a
 * multi-byte character produces something that is not text at all, and the
 * interpreters reverse `chars()`, so this walks the UTF-8 lead bytes and
 * copies each character whole. Malformed input is copied byte by byte rather
 * than rejected: a Keal string is bytes, and refusing here would make
 * `reversed` the only method that inspects them. */
KEAL_FN KealStr* keal_str_reversed(KealStr* s) {
    char* out = (char*)keal_alloc((size_t)s->len + 1);
    int64_t at = s->len;
    int64_t i = 0;
    while (i < s->len) {
        unsigned char lead = (unsigned char)s->bytes[i];
        int64_t width = 1;
        if (lead >= 0xF0) {
            width = 4;
        } else if (lead >= 0xE0) {
            width = 3;
        } else if (lead >= 0xC0) {
            width = 2;
        }
        if (i + width > s->len) {
            width = 1;
        }
        at -= width;
        memcpy(out + at, s->bytes + i, (size_t)width);
        i += width;
    }
    out[s->len] = '\0';
    return keal_str_owning(out, s->len);
}

KEAL_FN bool keal_write_file(KealStr* path, KealStr* content) {
    char cpath[4096];
    if (path->len >= (int64_t)sizeof cpath) {
        return false;
    }
    memcpy(cpath, path->bytes, (size_t)path->len);
    cpath[path->len] = '\0';
    FILE* f = keal_open(path, cpath, true);
    if (f == NULL) {
        return false;
    }
    size_t wrote = fwrite(content->bytes, 1, (size_t)content->len, f);
    int closed = fclose(f);
    return wrote == (size_t)content->len && closed == 0;
}

/* ---- the clock --------------------------------------------------------- */

/* Seconds since the Unix epoch, with whatever fraction the platform keeps —
 * the same number `SystemTime::now()` gives the two interpreters. UTC, and
 * only UTC: the prelude's calendar is written over this, and a local time
 * would mean reading `struct tm`, whose field order C does not fix. */
KEAL_FN double keal_time(void) {
#ifdef _WIN32
    /* 100-nanosecond ticks since 1601-01-01, which is 11644473600 seconds
     * before the Unix epoch. */
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    unsigned long long ticks =
        ((unsigned long long)ft.dwHighDateTime << 32) | ft.dwLowDateTime;
    return (double)ticks / 10000000.0 - 11644473600.0;
#else
    struct timespec ts;
    if (clock_gettime(CLOCK_REALTIME, &ts) != 0) {
        return 0.0;
    }
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1000000000.0;
#endif
}

/* Seconds east of UTC at a given instant — which depends on where the
 * machine thinks it is and on which side of a daylight-saving change the
 * instant falls, so it takes the instant rather than reading "now".
 *
 * The C standard says `struct tm` contains certain members but not in what
 * order, and the two interpreters call this same pair of functions from Rust
 * where a wrong guess about that order would be silent. So neither side ever
 * reads a field: the pointer goes straight into `strftime`, which prints the
 * offset as `+0200`, and only that string is read. Zero on any failure,
 * which is UTC — a true answer rather than a plausible wrong one. */
KEAL_FN int64_t keal_local_offset(int64_t at) {
    time_t when = (time_t)at;
    struct tm storage;
    struct tm* broken;
#ifdef _WIN32
    if (localtime_s(&storage, &when) != 0) {
        return 0;
    }
    broken = &storage;
#else
    broken = localtime_r(&when, &storage);
    if (broken == NULL) {
        return 0;
    }
#endif
    char text[8];
    if (strftime(text, sizeof text, "%z", broken) != 5) {
        return 0;
    }
    /* The sign is required, not merely allowed: a library answering `00200`
     * would otherwise be read as `+02:00` off the wrong two digits, and one
     * answering `Z` or nothing — which is what `%Z` gives where it has been
     * confused for `%z` — must fall through to UTC rather than to a
     * plausible number. */
    if (text[0] != '+' && text[0] != '-') {
        return 0;
    }
    for (int i = 1; i < 5; i++) {
        if (text[i] < '0' || text[i] > '9') {
            return 0;
        }
    }
    int64_t sign = text[0] == '-' ? -1 : 1;
    int64_t hours = (text[1] - '0') * 10 + (text[2] - '0');
    int64_t minutes = (text[3] - '0') * 10 + (text[4] - '0');
    return sign * (hours * 3600 + minutes * 60);
}

/* ---- numbers, randomness, and a line of input --------------------------- */

/* `abs` on an `Int`. Wraps at INT64_MIN exactly as Rust's `i64::abs` does in
 * a release build, through unsigned arithmetic — signed negation there is
 * undefined in C, and undefined is not a behaviour the three engines can
 * agree on. */
KEAL_FN int64_t keal_abs_i64(int64_t n) {
    return n < 0 ? (int64_t)(0u - (uint64_t)n) : n;
}

/* The interpreters' generator, algorithm for algorithm: xorshift64 with a
 * multiply, seeded from the clock on first use. The numbers differ from run
 * to run because the seed does; the shape of them does not differ between
 * engines, and a fixed seed would give the same sequence on all three. */
static uint64_t keal_rng_state = 0;

KEAL_FN double keal_random(void) {
    if (keal_rng_state == 0) {
        keal_rng_state = (uint64_t)(keal_time() * 1000000000.0) | 1u;
        if (keal_rng_state == 0) {
            keal_rng_state = 0x2545F4914F6CDD1DULL;
        }
    }
    keal_rng_state ^= keal_rng_state >> 12;
    keal_rng_state ^= keal_rng_state << 25;
    keal_rng_state ^= keal_rng_state >> 27;
    uint64_t scaled = (keal_rng_state * 0x2545F4914F6CDD1DULL) >> 11;
    return (double)scaled / (double)(1ULL << 53);
}

/* `randomInt(lo, hi)` — `lo` included, `hi` excluded, and an empty range is
 * the interpreters' error in the interpreters' words. */
KEAL_FN int64_t keal_random_int(int64_t lo, int64_t hi, int64_t line) {
    if (hi <= lo) {
        char msg[96];
        snprintf(msg, sizeof msg,
                 "randomInt(%" PRId64 ", %" PRId64 ") has an empty range", lo, hi);
        keal_panic(msg, line);
        return lo;
    }
    return lo + (int64_t)(keal_random() * (double)(hi - lo));
}

/* One line of standard input, without its ending, and NULL at end of file —
 * so `readLine() ?: ""` is how a program says it does not mind. Both a bare
 * newline and a carriage return before it are stripped, because a file
 * written on one platform is read on another. */
KEAL_FN KealStr* keal_read_line(void) {
    size_t cap = 128;
    size_t len = 0;
    char* buf = (char*)malloc(cap);
    if (buf == NULL) {
        return NULL;
    }
    int c;
    while ((c = fgetc(stdin)) != EOF && c != '\n') {
        if (len + 1 >= cap) {
            size_t grown = cap * 2;
            char* bigger = (char*)realloc(buf, grown);
            if (bigger == NULL) {
                free(buf);
                return NULL;
            }
            buf = bigger;
            cap = grown;
        }
        buf[len++] = (char)c;
    }
    if (c == EOF && len == 0) {
        free(buf);
        return NULL;
    }
    while (len > 0 && buf[len - 1] == '\r') {
        len--;
    }
    KealStr* out = keal_str_from_bytes(buf, (int64_t)len);
    free(buf);
    return out;
}

/* ---- the file system --------------------------------------------------- */

/* Four primitives, and no more: a name in the global namespace is reserved
 * for good and cannot ever be redefined, so only a system call earns one.
 * `exists`, `isFile`, `isDir` and `walkDir` are written over these in the
 * prelude, where a program that wants its own may shadow them. */

/* A path, as C wants it. False when it will not fit, which every caller
 * turns into the same absence a missing file gives. */
static bool keal_cpath(KealStr* path, char* out, size_t cap) {
    if (path->len < 0 || (size_t)path->len >= cap) {
        return false;
    }
    memcpy(out, path->bytes, (size_t)path->len);
    out[path->len] = '\0';
    return true;
}

/* Byte-lexicographic, which is what Rust's `sort` on a `String` is: the
 * three engines have to print one order, and a directory hands its entries
 * out in whatever order its file system pleases. */
static int keal_name_cmp(const void* a, const void* b) {
    KealStr* x = *(KealStr* const*)a;
    KealStr* y = *(KealStr* const*)b;
    int64_t n = x->len < y->len ? x->len : y->len;
    int c = n > 0 ? memcmp(x->bytes, y->bytes, (size_t)n) : 0;
    if (c != 0) {
        return c;
    }
    return x->len < y->len ? -1 : (x->len > y->len ? 1 : 0);
}

/* 0 nothing, 1 a file, 2 a directory. Deliberately an integer: the prelude
 * gives this three names, and this one stays the primitive. */
KEAL_FN int64_t keal_path_kind(KealStr* path) {
#ifdef _WIN32
    wchar_t* wp = keal_wpath(path);
    if (wp == NULL) {
        return 0;
    }
    DWORD attr = GetFileAttributesW(wp);
    free(wp);
    if (attr == INVALID_FILE_ATTRIBUTES) {
        return 0;
    }
    return (attr & FILE_ATTRIBUTE_DIRECTORY) ? 2 : 1;
#else
    char cpath[4096];
    if (!keal_cpath(path, cpath, sizeof cpath)) {
        return 0;
    }
    struct stat st;
    if (stat(cpath, &st) != 0) {
        return 0;
    }
    return S_ISDIR(st.st_mode) ? 2 : 1;
#endif
}

/* The entry names, sorted, without `.` and `..`. NULL when the path is not
 * a directory — the caller's `?:` decides what that means. */
KEAL_FN KealList* keal_list_dir(KealStr* path) {
    KealStr** names = NULL;
    int64_t count = 0;
    int64_t cap = 0;

#ifdef _WIN32
    /* The pattern is built in wide characters too: a directory whose own
     * name is not representable in the code page could not otherwise be
     * opened at all. */
    wchar_t* wp = keal_wpath(path);
    if (wp == NULL) {
        return NULL;
    }
    size_t wlen = wcslen(wp);
    wchar_t* pattern = (wchar_t*)malloc((wlen + 3) * sizeof(wchar_t));
    if (pattern == NULL) {
        free(wp);
        return NULL;
    }
    memcpy(pattern, wp, wlen * sizeof(wchar_t));
    pattern[wlen] = L'\\';
    pattern[wlen + 1] = L'*';
    pattern[wlen + 2] = L'\0';
    free(wp);
    WIN32_FIND_DATAW found;
    HANDLE h = FindFirstFileW(pattern, &found);
    free(pattern);
    if (h == INVALID_HANDLE_VALUE) {
        return NULL;
    }
    do {
        if (wcscmp(found.cFileName, L".") == 0 || wcscmp(found.cFileName, L"..") == 0) {
            continue;
        }
        KealStr* name = keal_narrow(found.cFileName);
#else
    char cpath[4096];
    if (!keal_cpath(path, cpath, sizeof cpath)) {
        return NULL;
    }
    DIR* d = opendir(cpath);
    if (d == NULL) {
        return NULL;
    }
    struct dirent* entry;
    while ((entry = readdir(d)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
            continue;
        }
        KealStr* name = keal_str_from_bytes(entry->d_name, (int64_t)strlen(entry->d_name));
#endif
        if (count == cap) {
            int64_t grown = cap == 0 ? 8 : cap * 2;
            KealStr** bigger = (KealStr**)realloc(names, (size_t)grown * sizeof *names);
            if (bigger == NULL) {
                keal_str_release(name);
                break;
            }
            names = bigger;
            cap = grown;
        }
        names[count++] = name;
#ifdef _WIN32
    } while (FindNextFileW(h, &found));
    FindClose(h);
#else
    }
    closedir(d);
#endif

    if (count > 1) {
        qsort(names, (size_t)count, sizeof *names, keal_name_cmp);
    }
    KealList* l = keal_list_new(rel_keal_str_thunk);
    for (int64_t i = 0; i < count; i++) {
        keal_list_push(l, (KealWord){ .p = names[i] });
    }
    free(names);
    return l;
}

/* Makes the directory and every parent it needs, and says true when the
 * directory is there afterwards — so making one twice is not a failure. */
KEAL_FN bool keal_make_dir(KealStr* path) {
#ifdef _WIN32
    /* Walked as wide characters. A separator is one code unit in UTF-16 and
     * cannot appear inside another character, so cutting the string at one
     * is safe in a way that cutting UTF-8 bytes would not be. */
    wchar_t* wp = keal_wpath(path);
    if (wp == NULL) {
        return false;
    }
    for (wchar_t* at = wp; *at != L'\0'; at++) {
        /* A leading separator is the root, and a repeated one is nothing. */
        if ((*at != L'/' && *at != L'\\') || at == wp || at[-1] == L'\0') {
            continue;
        }
        wchar_t was = *at;
        *at = L'\0';
        CreateDirectoryW(wp, NULL);
        *at = was;
    }
    CreateDirectoryW(wp, NULL);
    free(wp);
#else
    char cpath[4096];
    if (!keal_cpath(path, cpath, sizeof cpath)) {
        return false;
    }
    for (char* at = cpath; *at != '\0'; at++) {
        /* A leading separator is the root, and a repeated one is nothing. */
        if (*at != '/' || at == cpath || at[-1] == '\0') {
            continue;
        }
        char was = *at;
        *at = '\0';
        mkdir(cpath, 0777);
        *at = was;
    }
    mkdir(cpath, 0777);
#endif
    return keal_path_kind(path) == 2;
}

/* One file, or one empty directory. Not a tree: a recursive delete behind a
 * one-word name is how a program loses what it did not mean to. */
KEAL_FN bool keal_remove_path(KealStr* path) {
#ifdef _WIN32
    wchar_t* wp = keal_wpath(path);
    if (wp == NULL) {
        return false;
    }
    bool gone;
    if (keal_path_kind(path) == 2) {
        gone = RemoveDirectoryW(wp) != 0;
    } else {
        gone = DeleteFileW(wp) != 0;
    }
    free(wp);
    return gone;
#else
    char cpath[4096];
    if (!keal_cpath(path, cpath, sizeof cpath)) {
        return false;
    }
    if (keal_path_kind(path) == 2) {
        return rmdir(cpath) == 0;
    }
    return remove(cpath) == 0;
#endif
}

/* ---- running another program ------------------------------------------- */

/* `runCommand(argv)` — the exit code, the standard output and the standard
 * error, or NULL when the program could not be STARTED, which is a different
 * thing from a program that ran and failed. No shell: the list is the
 * argument vector, so a path with a space in it stays one argument.
 *
 * Two things here are not obvious and both were measured rather than
 * reasoned about. The first is that reading one stream to the end before the
 * other DEADLOCKS as soon as the child fills the other's pipe buffer — 4096
 * bytes on Windows, 65536 here — which is not a stress case but any command
 * that writes a result and a warning. So both streams are drained at once:
 * `poll` on this side, a thread per stream on the other. The second is that
 * on Windows the ANSI entry points appear to round-trip UTF-8 perfectly into
 * the child's own argv while the command line Windows actually built is
 * mojibake — correct in a test, wrong for any child that reads the wide
 * command line. So the wide entry points, and a conversion at the boundary.
 */

/* [exit code, standard output, standard error], the code as text because
 * that is the shape the interpreters return and the three have to agree. */
static KealList* keal_run_result(int64_t code, KealBuf* out, KealBuf* err) {
    char digits[24];
    snprintf(digits, sizeof digits, "%" PRId64, code);
    KealList* l = keal_list_new(rel_keal_str_thunk);
    keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(digits, (int64_t)strlen(digits)) });
    keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(out->data, out->len) });
    keal_list_push(l, (KealWord){ .p = keal_str_from_bytes(err->data, err->len) });
    return l;
}

#ifdef _WIN32

/* One argument, quoted the way `CommandLineToArgvW` unquotes. Backslashes
 * are only escapes next to a quote: a run before a literal quote is doubled
 * and the quote escaped, a run before the CLOSING quote is doubled, and
 * anything else is left alone. Without the second rule `"C:\dir\"` ends in an
 * escaped quote, the string never closes, and the next argument is swallowed. */
static void keal_win_quote(const wchar_t* arg, KealBuf* line) {
    size_t len = wcslen(arg);
    bool bare = len > 0;
    for (size_t i = 0; i < len; i++) {
        if (arg[i] == L' ' || arg[i] == L'\t' || arg[i] == L'"') {
            bare = false;
            break;
        }
    }
    if (bare) {
        keal_buf_bytes(line, (const char*)arg, (int64_t)(len * sizeof(wchar_t)));
        return;
    }
    wchar_t q = L'"';
    keal_buf_bytes(line, (const char*)&q, (int64_t)sizeof q);
    for (size_t i = 0; i < len;) {
        size_t slashes = 0;
        while (i < len && arg[i] == L'\\') {
            slashes++;
            i++;
        }
        wchar_t bs = L'\\';
        if (i == len) {
            for (size_t k = 0; k < slashes * 2; k++) {
                keal_buf_bytes(line, (const char*)&bs, (int64_t)sizeof bs);
            }
            break;
        }
        if (arg[i] == L'"') {
            for (size_t k = 0; k < slashes * 2 + 1; k++) {
                keal_buf_bytes(line, (const char*)&bs, (int64_t)sizeof bs);
            }
        } else {
            for (size_t k = 0; k < slashes; k++) {
                keal_buf_bytes(line, (const char*)&bs, (int64_t)sizeof bs);
            }
        }
        keal_buf_bytes(line, (const char*)&arg[i], (int64_t)sizeof(wchar_t));
        i++;
    }
    keal_buf_bytes(line, (const char*)&q, (int64_t)sizeof q);
}

static wchar_t* keal_widen(const char* utf8, int64_t len) {
    int n = MultiByteToWideChar(CP_UTF8, 0, utf8, (int)len, NULL, 0);
    wchar_t* w = (wchar_t*)malloc(((size_t)n + 1) * sizeof(wchar_t));
    if (w == NULL) {
        return NULL;
    }
    if (n > 0) {
        MultiByteToWideChar(CP_UTF8, 0, utf8, (int)len, w, n);
    }
    w[n] = L'\0';
    return w;
}

typedef struct {
    HANDLE h;
    KealBuf buf;
} KealReader;

static DWORD WINAPI keal_drain(LPVOID arg) {
    KealReader* r = (KealReader*)arg;
    char chunk[16384];
    DWORD got;
    while (ReadFile(r->h, chunk, sizeof chunk, &got, NULL) && got > 0) {
        keal_buf_bytes(&r->buf, chunk, (int64_t)got);
    }
    return 0;
}

KEAL_FN KealList* keal_run_command(KealList* argv) {
    if (argv->len == 0) {
        return NULL;
    }
    /* The command line, as wide characters, program included as token zero.
     *
     * That token is ALL the naming CreateProcessW gets: lpApplicationName
     * below is NULL on purpose. Given a name there, Windows completes it with
     * the current drive and directory and stops — it never consults the
     * search path — so `runCommand(["git", ...])` failed to start on Windows
     * while both interpreters, which go through execvp's POSIX equivalent in
     * Rust, resolved it. The failure wore the language's ordinary answer for
     * an absent command, `null`, so a program could not tell "not installed"
     * from "installed and unreachable".
     *
     * Passing NULL moves the naming into the line, which is the form that
     * searches. What lpApplicationName bought was the unquoted-path
     * ambiguity: `C:\Program Files\x.exe` unquoted invites Windows to try
     * `C:\Program.exe` first. keal_win_quote already quotes any token holding
     * a space, so that protection is kept where it was, one layer down. */
    KealBuf line;
    keal_buf_init(&line);
    for (int64_t i = 0; i < argv->len; i++) {
        KealStr* a = (KealStr*)argv->data[i].p;
        wchar_t* w = keal_widen(a->bytes, a->len);
        if (w == NULL) {
            free(line.data);
            return NULL;
        }
        if (i > 0) {
            wchar_t sp = L' ';
            keal_buf_bytes(&line, (const char*)&sp, (int64_t)sizeof sp);
        }
        keal_win_quote(w, &line);
        free(w);
    }
    wchar_t nul = L'\0';
    keal_buf_bytes(&line, (const char*)&nul, (int64_t)sizeof nul);

    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof sa;
    sa.lpSecurityDescriptor = NULL;
    sa.bInheritHandle = TRUE;
    HANDLE or_ = NULL, ow = NULL, er = NULL, ew = NULL, nulh = INVALID_HANDLE_VALUE;
    if (!CreatePipe(&or_, &ow, &sa, 0) || !CreatePipe(&er, &ew, &sa, 0)) {
        goto fail;
    }
    /* The parent's read ends must not be inherited, or the child holds them
     * open and ReadFile never reaches end of file. */
    SetHandleInformation(or_, HANDLE_FLAG_INHERIT, 0);
    SetHandleInformation(er, HANDLE_FLAG_INHERIT, 0);
    nulh = CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE,
                       &sa, OPEN_EXISTING, 0, NULL);

    STARTUPINFOW si;
    ZeroMemory(&si, sizeof si);
    si.cb = sizeof si;
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = nulh;
    si.hStdOutput = ow;
    si.hStdError = ew;
    PROCESS_INFORMATION pi;
    ZeroMemory(&pi, sizeof pi);
    if (!CreateProcessW(NULL, (wchar_t*)line.data, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi)) {
        goto fail;
    }
    /* And the parent must drop the write ends, or end of file never comes. */
    CloseHandle(ow);
    ow = NULL;
    CloseHandle(ew);
    ew = NULL;
    if (nulh != INVALID_HANDLE_VALUE) {
        CloseHandle(nulh);
        nulh = INVALID_HANDLE_VALUE;
    }

    KealReader ro;
    KealReader re;
    ro.h = or_;
    re.h = er;
    keal_buf_init(&ro.buf);
    keal_buf_init(&re.buf);
    HANDLE t1 = CreateThread(NULL, 0, keal_drain, &ro, 0, NULL);
    HANDLE t2 = CreateThread(NULL, 0, keal_drain, &re, 0, NULL);
    WaitForSingleObject(pi.hProcess, INFINITE);
    if (t1 != NULL) {
        WaitForSingleObject(t1, INFINITE);
        CloseHandle(t1);
    }
    if (t2 != NULL) {
        WaitForSingleObject(t2, INFINITE);
        CloseHandle(t2);
    }
    DWORD code = 0;
    GetExitCodeProcess(pi.hProcess, &code);
    /* An image can RESOLVE and then fail to LOAD: CreateProcessW succeeds,
     * the process object exists, and the child dies before its entry point
     * runs. A missing DLL is the common way. On this side that is a nonzero
     * exit code; on POSIX the very same failure — a missing shared object —
     * makes `execve` fail, so the errno pipe above reports it as "could not
     * be started" and the caller gets null. Mapping the two load-time
     * statuses to null is not a heuristic, then: it is what makes the two
     * platforms answer the same question the same way.
     *
     * A crash is NOT included. 0xC0000005 and its neighbours mean the program
     * ran and then died, which is a failure and not an absence, and the
     * caller is owed the difference. */
    if (code == 0xC0000135u || code == 0xC0000142u) {
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        CloseHandle(or_);
        CloseHandle(er);
        free(line.data);
        free(ro.buf.data);
        free(re.buf.data);
        return NULL;
    }
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    CloseHandle(or_);
    CloseHandle(er);
    free(line.data);
    KealList* result = keal_run_result((int64_t)(int32_t)code, &ro.buf, &re.buf);
    free(ro.buf.data);
    free(re.buf.data);
    return result;

fail:
    if (or_) CloseHandle(or_);
    if (ow) CloseHandle(ow);
    if (er) CloseHandle(er);
    if (ew) CloseHandle(ew);
    if (nulh != INVALID_HANDLE_VALUE) CloseHandle(nulh);
    free(line.data);
    return NULL;
}

#else

KEAL_FN KealList* keal_run_command(KealList* argv) {
    if (argv->len == 0) {
        return NULL;
    }
    char** words = (char**)calloc((size_t)argv->len + 1, sizeof(char*));
    if (words == NULL) {
        return NULL;
    }
    for (int64_t i = 0; i < argv->len; i++) {
        KealStr* a = (KealStr*)argv->data[i].p;
        words[i] = (char*)malloc((size_t)a->len + 1);
        if (words[i] == NULL) {
            for (int64_t k = 0; k < i; k++) { free(words[k]); }
            free(words);
            return NULL;
        }
        memcpy(words[i], a->bytes, (size_t)a->len);
        words[i][a->len] = '\0';
    }

    int outp[2], errp[2], failp[2];
    if (pipe(outp) != 0) { goto cleanup_words; }
    if (pipe(errp) != 0) { close(outp[0]); close(outp[1]); goto cleanup_words; }
    /* How the child says `exec` failed. Closed automatically when exec
     * succeeds, so an empty read is success — the same way the interpreters
     * tell "could not start" from "started and failed". */
    if (pipe(failp) != 0) {
        close(outp[0]); close(outp[1]); close(errp[0]); close(errp[1]);
        goto cleanup_words;
    }
    fcntl(failp[1], F_SETFD, FD_CLOEXEC);

    pid_t pid = fork();
    if (pid < 0) {
        close(outp[0]); close(outp[1]); close(errp[0]); close(errp[1]);
        close(failp[0]); close(failp[1]);
        goto cleanup_words;
    }
    if (pid == 0) {
        close(outp[0]);
        close(errp[0]);
        close(failp[0]);
        dup2(outp[1], STDOUT_FILENO);
        dup2(errp[1], STDERR_FILENO);
        close(outp[1]);
        close(errp[1]);
        execvp(words[0], words);
        int e = errno;
        ssize_t ignored = write(failp[1], &e, sizeof e);
        (void)ignored;
        _exit(127);
    }

    close(outp[1]);
    close(errp[1]);
    close(failp[1]);

    KealBuf out;
    KealBuf err;
    keal_buf_init(&out);
    keal_buf_init(&err);
    /* Both streams drained together. Taking one to the end first deadlocks
     * the moment the child fills the other's buffer, which is any command
     * that writes a result and a warning. */
    struct pollfd fds[2];
    fds[0].fd = outp[0];
    fds[1].fd = errp[0];
    fds[0].events = fds[1].events = POLLIN;
    int open_streams = 2;
    while (open_streams > 0) {
        fds[0].revents = fds[1].revents = 0;
        if (poll(fds, 2, -1) < 0) {
            if (errno == EINTR) { continue; }
            break;
        }
        for (int i = 0; i < 2; i++) {
            if (fds[i].fd < 0 || fds[i].revents == 0) {
                continue;
            }
            char chunk[16384];
            ssize_t got = read(fds[i].fd, chunk, sizeof chunk);
            if (got > 0) {
                keal_buf_bytes(i == 0 ? &out : &err, chunk, (int64_t)got);
            } else if (got == 0 || errno != EINTR) {
                close(fds[i].fd);
                fds[i].fd = -1;
                open_streams--;
            }
        }
    }

    int failed = 0;
    ssize_t got_fail = read(failp[0], &failed, sizeof failed);
    close(failp[0]);

    int status = 0;
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {
    }
    for (int64_t i = 0; i < argv->len; i++) { free(words[i]); }
    free(words);

    if (got_fail == (ssize_t)sizeof failed) {
        /* `exec` never ran the program, so nothing started. */
        free(out.data);
        free(err.data);
        return NULL;
    }
    int64_t code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
    KealList* result = keal_run_result(code, &out, &err);
    free(out.data);
    free(err.data);
    return result;

cleanup_words:
    for (int64_t i = 0; i < argv->len; i++) { free(words[i]); }
    free(words);
    return NULL;
}

#endif

/* ---- checked integer arithmetic --------------------------------------- */

/* The interpreter and the VM both refuse to wrap, so native code must not
 * either: a program that overflows should fail the same way whichever engine
 * runs it. */
KEAL_FN int64_t keal_add(int64_t a, int64_t b, int64_t line) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) {
        keal_panic("integer overflow", line);
        return 0;
    }
    return r;
}

KEAL_FN int64_t keal_sub(int64_t a, int64_t b, int64_t line) {
    int64_t r;
    if (__builtin_sub_overflow(a, b, &r)) {
        keal_panic("integer overflow", line);
        return 0;
    }
    return r;
}

KEAL_FN int64_t keal_mul(int64_t a, int64_t b, int64_t line) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) {
        keal_panic("integer overflow", line);
        return 0;
    }
    return r;
}

KEAL_FN int64_t keal_div(int64_t a, int64_t b, int64_t line) {
    if (b == 0) {
        keal_panic("division by zero", line);
        return 0;
    }
    if (a == INT64_MIN && b == -1) {
        keal_panic("integer overflow", line);
        return 0;
    }
    return a / b;
}

/* The shifts.
 *
 * An Int has 64 bits, so 0 through 63 name a shift and everything else names
 * a mistake — which panics, rather than being left undefined the way C
 * leaves it. Every alternative reading a language could pick instead (clamp,
 * count modulo 64, saturate) turns a bug into a number the program carries.
 *
 * `shl` truncates: the bits that leave the top are gone. It is the one place
 * in Keal where a value is not checked, and it is deliberate — these
 * operators are defined on the 64 bits of an Int, not on the number those
 * bits spell.
 *
 * All three go through uint64_t: shifting a signed value past its sign bit
 * is undefined in C, and `>>` on a negative one is implementation-defined. */
KEAL_FN bool keal_shift_ok(int64_t by, const char* op, int64_t line) {
    if (by < 0) {
        char msg[96];
        snprintf(msg, sizeof msg, "`%s` needs a shift count of 0 or more, got %" PRId64, op, by);
        keal_panic(msg, line);
        return false;
    }
    if (by > 63) {
        char msg[96];
        snprintf(msg, sizeof msg, "`%s` cannot shift an Int by %" PRId64 ": it has 64 bits", op, by);
        keal_panic(msg, line);
        return false;
    }
    return true;
}

KEAL_FN int64_t keal_shl(int64_t a, int64_t b, int64_t line) {
    if (!keal_shift_ok(b, "shl", line)) { return 0; }
    return (int64_t)((uint64_t)a << b);
}

KEAL_FN int64_t keal_shr(int64_t a, int64_t b, int64_t line) {
    if (!keal_shift_ok(b, "shr", line)) { return 0; }
    /* Arithmetic: the sign is carried in at the top, so `-8 shr 1` is `-4`
     * the way `-8 / 2` is. */
    if (a < 0) {
        return (int64_t)~((~(uint64_t)a) >> b);
    }
    return (int64_t)((uint64_t)a >> b);
}

KEAL_FN int64_t keal_ushr(int64_t a, int64_t b, int64_t line) {
    if (!keal_shift_ok(b, "ushr", line)) { return 0; }
    return (int64_t)((uint64_t)a >> b);
}

KEAL_FN int64_t keal_rem(int64_t a, int64_t b, int64_t line) {
    if (b == 0) {
        keal_panic("remainder by zero", line);
        return 0;
    }
    if (a == INT64_MIN && b == -1) {
        return 0;
    }
    return a % b;
}

/* ---- the drop hook ---------------------------------------------------- */

/* Objects whose last reference died wait here, whole and holding one
 * resurrected reference, for the next statement boundary — where their
 * class's `drop` runs and the release resumes. FIFO: death order is drop
 * order. A `drop` body's own deaths join the same sweep; the guard stops
 * the sweep from recursing into itself. */
typedef struct KealPendingDrop {
    void* obj;
    void (*run)(void*);
    struct KealPendingDrop* next;
} KealPendingDrop;

static _Thread_local KealPendingDrop* keal_drops_head = NULL;
static _Thread_local KealPendingDrop* keal_drops_tail = NULL;
static _Thread_local bool keal_draining = false;

KEAL_FN void keal_queue_drop(void* obj, void (*run)(void*)) {
    KealPendingDrop* n = (KealPendingDrop*)keal_alloc(sizeof(KealPendingDrop));
    n->obj = obj;
    n->run = run;
    n->next = NULL;
    if (keal_drops_tail == NULL) {
        keal_drops_head = n;
    } else {
        keal_drops_tail->next = n;
    }
    keal_drops_tail = n;
}

KEAL_FN void keal_drain_drops(void) {
    if (keal_draining) {
        return;
    }
    keal_draining = true;
    while (keal_drops_head != NULL) {
        KealPendingDrop* n = keal_drops_head;
        keal_drops_head = n->next;
        if (keal_drops_head == NULL) {
            keal_drops_tail = NULL;
        }
        void (*run)(void*) = n->run;
        void* obj = n->obj;
        free(n);
        run(obj);
        /* A panic inside a `drop` stops the sweep; what remains drains
         * at the next boundary. */
        if (keal_unwinding) {
            break;
        }
    }
    keal_draining = false;
}

/* The thrown value itself, live only while `keal_unwind_has_value`. */
static _Thread_local KealAny keal_unwind_value;

/* A `throw` of a value. The value crosses whole — tag and payload, the
 * `Any` machinery — and its rendering is the message a `catch (e)` reads,
 * the same text the interpreters give since both take it from `display`.
 * Uncaught, this ends the program printing that message, like any panic. */
KEAL_FN void keal_throw_value(KealAny v, int64_t line) {
    KealStr* m = keal_any_display(v);
    /* Only a fresh unwind adopts the value: one already unwinding keeps
     * the first, which is the rule the message already follows. */
    bool fresh = keal_try_depth > 0 && !keal_unwinding;
    keal_panic(m->bytes, line);
    if (fresh) {
        keal_unwind_value = keal_any_retain(v);
        keal_unwind_has_value = true;
    }
    keal_str_release(m);
}

/* Does the value being unwound answer to this tag? A message-only unwind
 * answers to `String`, which is what it is. */
KEAL_FN bool keal_unwind_is(const KealTypeInfo* ti) {
    return keal_unwind_has_value ? keal_unwind_value.ti == ti : ti == &keal_ti_str;
}

/* `catch (e: Any)`: anything but null, as `is Any` reads it everywhere. */
KEAL_FN bool keal_unwind_is_any(void) {
    return !keal_unwind_has_value || keal_unwind_value.ti != NULL;
}

/* Ends the unwind at a typed `catch`: hands the value over, owned. */
KEAL_FN KealAny keal_unwind_value_take(void) {
    keal_unwinding = false;
    if (!keal_unwind_has_value) {
        KealAny a;
        a.ti = &keal_ti_str;
        a.w.p = keal_str_from_bytes(keal_unwind_msg, (int64_t)strlen(keal_unwind_msg));
        return a;
    }
    keal_unwind_has_value = false;
    return keal_unwind_value;
}

/* Ends the unwind at a `catch`: hands the message over and clears the
 * flag, so the handler runs like any other code. A value the clause did
 * not ask for dies here — the message is all it wanted. */
KEAL_FN KealStr* keal_unwind_take(void) {
    keal_unwinding = false;
    if (keal_unwind_has_value) {
        keal_unwind_has_value = false;
        keal_any_release(keal_unwind_value);
    }
    return keal_str_from_bytes(keal_unwind_msg, (int64_t)strlen(keal_unwind_msg));
}
