/* The C spelling of what godemo.go exports. cgo's generated header says
 * the same thing in Go typedefs (GoInt64, char*); this hand-written one
 * says it in the exact C types `keal bindgen` binds — same ABI, and the
 * `const` on the borrowed string is a promise Go keeps (GoString copies).
 */
#include <stdint.h>
#include <stdbool.h>

int64_t go_fib(int64_t n);
double go_hypot(double a, double b);
bool go_even(int64_t n);
char* go_banner(void);
char* go_shout(const char* text);
