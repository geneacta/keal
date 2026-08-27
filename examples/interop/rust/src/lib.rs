//! The Rust half of the demo: plain `extern "C"` exports, nothing else.

use std::ffi::{c_char, CStr, CString};

#[no_mangle]
pub extern "C" fn rust_fib(n: i64) -> i64 {
    if n < 2 { n } else { rust_fib(n - 1) + rust_fib(n - 2) }
}

/// A borrowed string in, an owned (malloc'd) string out — exactly the
/// `borrow String` / `own String` contract.
#[no_mangle]
pub extern "C" fn rust_greet(name: *const c_char) -> *mut c_char {
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    CString::new(format!("hello from Rust, {}", name)).unwrap().into_raw()
}
