#!/bin/sh
# One Keal file calling C, C++, Rust, Go, Java and Kotlin. Each language
# contributes what it naturally exports; Keal binds all of it.
set -e
cd "$(dirname "$0")"

(cd ../rust && cargo build --release)                            # Rust
(cd ../go && go build -buildmode=c-archive -o libgodemo.a godemo.go)  # Go
kotlinc kotlin/Greeter.kt -include-runtime -d kotlin/greeter.jar # Kotlin

keal bindgen poly.h > bindings.keal                              # C ABI side

JH=$(/usr/libexec/java_home)
CLASSPATH=kotlin/greeter.jar keal build main.keal \
    native.c native.cpp \
    ../rust/target/release/libkealdemo.a ../go/libgodemo.a \
    -I. -I$JH/include -I$JH/include/darwin \
    -L$JH/lib/server -ljvm -Wl,-rpath,$JH/lib/server

./main
