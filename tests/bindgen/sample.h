/* A sample of everything bindgen understands, and everything it refuses. */
#ifndef SAMPLE_H
#define SAMPLE_H

#include <stdint.h>
#include <stdbool.h>

#ifndef KEAL_MIRROR_Vec2
#define KEAL_MIRROR_Vec2
typedef struct Keal_Vec2 {
    double x;
    double y;
} Keal_Vec2;
#endif

/* Clean crossings. */
int64_t add64(int64_t a, int64_t b);
extern long long triple(long long n);
double scale(double x, double factor);
bool flag_of(int64_t n);
int64_t count_vowels(const char *text);
char *shout(const char *text);
void reset(void);
void tick();
double vec2_dot(Keal_Vec2 a, Keal_Vec2 b);
Keal_Vec2 vec2_scale(Keal_Vec2 v, double k);
int64_t unnamed_params(int64_t, double);

/* Refused, each with its reason. */
int plain_int(int n);
float small_float(float f);
const char *version(void);
void fill(char *buffer);
int64_t sum_all(int64_t first, ...);
void on_each(int64_t (*cb)(int64_t));
static int64_t hidden(int64_t n);
unsigned int mask(void);

/* Not functions at all: ignored silently. */
extern int64_t some_global;
struct Opaque;

#endif
