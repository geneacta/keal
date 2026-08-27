// A Go static library Keal links against. The whole path is the C ABI:
// cgo exports it (`go build -buildmode=c-archive`), Keal's output *is* C.
package main

import "C"

import (
	"fmt"
	"math"
	"strings"
)

//export go_fib
func go_fib(n int64) int64 {
	a, b := int64(0), int64(1)
	for i := int64(0); i < n; i++ {
		a, b = b, a+b
	}
	return a
}

//export go_hypot
func go_hypot(a float64, b float64) float64 {
	return math.Hypot(a, b)
}

//export go_even
func go_even(n int64) bool {
	return n%2 == 0
}

//export go_banner
func go_banner() *C.char {
	// C.CString allocates with the C allocator — exactly what Keal's
	// `own String` promises to free.
	return C.CString(fmt.Sprintf("Go %s reporting", "c-archive"))
}

//export go_shout
func go_shout(text *C.char) *C.char {
	return C.CString(strings.ToUpper(C.GoString(text)) + "!")
}

func main() {}
