#!/usr/bin/env python3
# Copy `src/runtime.c` into `selfhost/runtimesrc.keal`.
#
# The self-hosted emitter carries the runtime as a string so that it needs no
# path to the Rust tree, and the emitter oracle test compares the two whole.
# So the copy is not a convenience: a `runtime.c` edited without it makes the
# twins disagree about a file neither of them wrote. This script exists
# because that was learned twice in one afternoon.
#
# Nothing is escaped, and that is checked rather than assumed: the text goes
# inside a triple-quoted Keal string, so a runtime containing that delimiter
# would end the string early. It is refused here, by line number, which is a
# better answer than C that will not parse.
#
# A dollar-brace needs no check, and the reason is worth writing down because
# the obvious guess is wrong: a triple-quoted Keal string is RAW, and
# `runtime.c` has carried `${...}` inside a comment for as long as this file
# has existed.

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUNTIME = ROOT / "src" / "runtime.c"
TWIN = ROOT / "selfhost" / "runtimesrc.keal"
FENCE = chr(34) * 3

text = RUNTIME.read_text()
for n, line in enumerate(text.splitlines(), 1):
    if FENCE in line:
        sys.exit("src/runtime.c:%d: `%s` would end the embedded string\n  %s"
                 % (n, FENCE, line.strip()))

old = TWIN.read_text()
head, rest = old.split("return " + FENCE, 1)
_, tail = rest.rsplit(FENCE, 1)
new = head + "return " + FENCE + text + FENCE + tail
if new == old:
    print("selfhost/runtimesrc.keal already matches src/runtime.c")
else:
    TWIN.write_text(new)
    print("selfhost/runtimesrc.keal <- src/runtime.c (%d lines)" % len(text.splitlines()))
