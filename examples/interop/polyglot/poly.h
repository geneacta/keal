/* One header, four native languages: what each exports, spelled in the
 * exact C types `keal bindgen` binds. The Rust and Go prototypes repeat
 * their own demos' headers; the linker resolves them from the archives. */
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

int64_t c_add(int64_t a, int64_t b);           /* native.c    (C)    */
char* cpp_shout(const char* text);             /* native.cpp  (C++)  */
int64_t rust_fib(int64_t n);                   /* ../rust     (Rust) */
char* rust_greet(const char* name);
double go_hypot(double a, double b);           /* ../go       (Go)   */
char* go_shout(const char* text);

#ifdef __cplusplus
}
#endif
