/* What `cbindgen --lang c` produces for src/lib.rs (checked in so the demo
 * reads without running cbindgen). */
#include <stdint.h>

int64_t rust_fib(int64_t n);
char *rust_greet(const char *name);
