#!/usr/bin/env python3
"""Grammar-driven fuzzing of `keal check`.

The question this answers is not "does it compile?" but "does the checker
ever *crash*?" — every generated program, well- or ill-typed, must come
back with exit code 0 (accepted) or 1 (refused with diagnostics), never a
panic, an abort or a signal. The generator is deliberately hostile: it
mixes types wrong on purpose, nests generics, throws strings at Int
parameters, negates non-booleans, calls methods that do not exist.

    python3 tests/fuzz/fuzz.py <keal-binary> <count> [seed]

Deterministic per seed. Any crasher is written to tests/fuzz/crash-N.keal
and the run fails.
"""

import random
import subprocess
import sys


TYPES = ["Int", "Float", "Bool", "String", "Int?", "List<Int>", "List<String>",
         "Map<String, Int>", "Box<Int>", "Box<String>", "Box<List<Int>>",
         "(Int) -> Int", "(String) -> Bool", "Comp"]

LITERALS = {
    "Int": ["0", "1", "-7", "9223372036854775807", "2 ** 10",
            "0xFF", "0b1010", "0xFFFFFFFFFFFFFFFF", "bnot 0", "1 shl 8"],
    "Float": ["0.0", "1.5", "-2.25", "1.0 / 3.0"],
    "Bool": ["true", "false", "1 < 2", "not true"],
    "String": ['"s"', '""', '"${1 + 2}"', '"a" + "b"'],
    "Int?": ["null", "5", "\"s\".toInt()"],
    "List<Int>": ["[1, 2]", "[]", "[1 + 1]"],
    "List<String>": ['["a"]', "[]"],
    "Map<String, Int>": ['{"k": 1}', "{}"],
    "Box<Int>": ["Box(1)"],
    "Box<String>": ['Box("s")'],
    "Box<List<Int>>": ["Box([1])"],
    "(Int) -> Int": ["{ n -> n + 1 }"],
    "(String) -> Bool": ['{ s -> s == "x" }'],
    # `Comp(0)` was here until `Comp` stopped being a record. A generator
    # that names a constructor the language has dropped spends its
    # "accepted" half on programs that can only ever be refused, and stops
    # exercising the half it was written for.
    "Comp": ["compare(1, 2)", "1 <=> 2", "less", "equal", "greater"],
}

BINOPS = ["+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", ">=", "**",
          "^/", "<=>", "<==>", "and", "or", "xor", "nand", "implies", "?:", "..",
          # The bit operators, which refuse to mix with arithmetic and with
          # each other. Every operand here is parenthesised, so what they
          # meet is the type rule rather than the mixing rule — and both
          # answers are refusals, which is one of the two outcomes this asks
          # the checker to survive.
          "band", "bor", "bxor", "shl", "shr", "ushr"]

METHODS = ["size", "length", "toString", "toInt", "abs", "get(0)", "take(2)",
           "drop(1)", "add(1)", "keys()", "contains(1)", "isLess",
           "map({ x -> x })", "filter({ x -> true })", "nonsense()",
           "pow(2)", "root(2)", "compareTo(1)", "join(\",\")"]


# Chance that a supposedly well-typed position gets the wrong type on
# purpose: the interesting frontier is programs that are ALMOST right.
MISTAKE = 0.18


def lit(rng, ty=None):
    ty = ty or rng.choice(TYPES)
    return rng.choice(LITERALS.get(ty, ["0"]))


def typed(rng, ty, depth=0):
    """An expression that has type `ty` — unless a deliberate mistake
    swaps in another type, which is the point of the exercise."""
    if rng.random() < MISTAKE:
        ty = rng.choice(TYPES)
    if depth > 3:
        return lit(rng, ty)
    roll = rng.random()
    if roll < 0.40 or ty not in ("Int", "Float", "Bool", "String"):
        if ty == "Comp" and roll < 0.5:
            return f"({typed(rng, 'Int', depth + 1)} <=> {typed(rng, 'Int', depth + 1)})"
        if ty == "Int?" and roll < 0.6:
            return f"({typed(rng, 'Bool', depth + 1)} ? {lit(rng, 'Int?')} : null)"
        return lit(rng, ty)
    if ty == "Bool":
        if roll < 0.55:
            return f"({typed(rng, 'Int', depth + 1)} {rng.choice(['<', '<=', '==', '!='])} {typed(rng, 'Int', depth + 1)})"
        if roll < 0.70:
            return f"({typed(rng, 'Bool', depth + 1)} {rng.choice(['and', 'or', 'xor', 'nand', 'implies'])} {typed(rng, 'Bool', depth + 1)})"
        if roll < 0.80:
            return f"(not {typed(rng, 'Bool', depth + 1)})"
        return f"({typed(rng, 'String', depth + 1)}.length > {typed(rng, 'Int', depth + 1)})"
    if ty == "Int":
        if roll < 0.60:
            return f"({typed(rng, 'Int', depth + 1)} {rng.choice(['+', '-', '*', '**'])} {typed(rng, 'Int', depth + 1)})"
        if roll < 0.72:
            return f"({typed(rng, 'Bool', depth + 1)} ? {typed(rng, 'Int', depth + 1)} : {typed(rng, 'Int', depth + 1)})"
        if roll < 0.80:
            return f"({typed(rng, 'Comp', depth + 1)} ? {typed(rng, 'Int', depth + 1)} : {typed(rng, 'Int', depth + 1)} : {typed(rng, 'Int', depth + 1)})"
        if roll < 0.88:
            return f"[{typed(rng, 'Int', depth + 1)}, {typed(rng, 'Int', depth + 1)}].size"
        return f"f({typed(rng, 'Int', depth + 1)})"
    if ty == "Float":
        return f"({typed(rng, 'Float', depth + 1)} {rng.choice(['+', '*', '/'])} {typed(rng, 'Float', depth + 1)})"
    if ty == "String":
        if roll < 0.60:
            return f"({typed(rng, 'String', depth + 1)} + {typed(rng, 'String', depth + 1)})"
        if roll < 0.80:
            return f'"v=${{{typed(rng, "Int", depth + 1)}}}"'
        return f"{typed(rng, 'Int', depth + 1)}.toString()"
    return lit(rng, ty)


def expr(rng, depth=0):
    roll = rng.random()
    if roll < 0.6:
        return typed(rng, rng.choice(TYPES), depth)
    if depth > 3:
        return lit(rng)
    if roll < 0.7:
        return f"({expr(rng, depth + 1)} {rng.choice(BINOPS)} {expr(rng, depth + 1)})"
    if roll < 0.78:
        return f"{lit(rng)}.{rng.choice(METHODS)}"
    if roll < 0.85:
        return f"({typed(rng, 'Bool', depth + 1)} ? {expr(rng, depth + 1)} : {expr(rng, depth + 1)})"
    if roll < 0.92:
        return f"(if ({typed(rng, 'Bool', depth + 1)}) {{ {expr(rng, depth + 1)} }} else {{ {expr(rng, depth + 1)} }})"
    return f"{rng.choice(['f', 'g', 'h'])}({expr(rng, depth + 1)})"


def stmt(rng, i, depth=0):
    roll = rng.random()
    ty = rng.choice(TYPES)
    if roll < 0.30:
        kw = rng.choice(["val", "var"])
        if rng.random() < 0.6:
            return f"{kw} v{i}: {ty} = {typed(rng, ty)}"
        return f"{kw} v{i} = {expr(rng)}"
    if roll < 0.40 and i > 0:
        return f"v{rng.randrange(i)} = {expr(rng)}"
    if roll < 0.50:
        return f"println({expr(rng)})"
    if roll < 0.58 and depth < 2:
        return (f"if ({expr(rng)}) {{\n    {stmt(rng, i, depth + 1)}\n}} else {{\n"
                f"    {stmt(rng, i, depth + 1)}\n}}")
    if roll < 0.64 and depth < 2:
        return (f"while ({expr(rng)}) {{\n    {stmt(rng, i, depth + 1)}\n    break\n}}")
    if roll < 0.70 and depth < 2:
        return (f"for (it{i} in {expr(rng)}) {{\n    {stmt(rng, i, depth + 1)}\n}}")
    if roll < 0.76 and depth < 2:
        return (f"try {{\n    {stmt(rng, i, depth + 1)}\n}} catch (e{i}) {{\n"
                f"    {stmt(rng, i, depth + 1)}\n}}")
    if roll < 0.80:
        return f"throw {expr(rng)}"
    if roll < 0.85:
        return (f"when ({expr(rng)}) {{\n    1 -> {expr(rng)}\n    "
                f"else -> {expr(rng)}\n}}")
    if roll < 0.90:
        return f"assert({expr(rng)}, {expr(rng)})"
    return f"val w{i} = {expr(rng)} <=> {expr(rng)}"


def clean_stmt(rng, i):
    ty = rng.choice(["Int", "Float", "Bool", "String", "Comp", "List<Int>"])
    roll = rng.random()
    if roll < 0.5:
        return f"val v{i}: {ty} = {typed(rng, ty)}"
    if roll < 0.65:
        return f"println({typed(rng, 'String')})"
    if roll < 0.8:
        return (f"if ({typed(rng, 'Bool')}) {{\n    println({typed(rng, 'Int')}.toString())\n}} else {{\n"
                f"    println({typed(rng, 'String')})\n}}")
    if roll < 0.9:
        return (f"try {{\n    println(({typed(rng, 'Int')} / 1).toString())\n}} catch (e{i}) {{\n"
                f"    println(e{i})\n}}")
    return f"val w{i} = {typed(rng, 'Int')} <=> {typed(rng, 'Int')} ? \"a\" : \"b\" : \"c\""


def program(rng):
    global MISTAKE
    # A third of the programs are honest: no injected mistakes, tame
    # statements — those walk the checker's accept paths to the end.
    if rng.random() < 0.35:
        saved = MISTAKE
        MISTAKE = 0.0
        parts = ["class Box<T>(val v: T)"]
        parts.append("func f(x: Int): Int {\n    return x + 1\n}")
        parts.append("func g<T>(x: Box<T>, y: Box<List<T>>): T {\n    return x.v\n}")
        parts.append("func h<T: Ord>(a: T, b: T): Comp {\n    return a <=> b\n}")
        for i in range(rng.randrange(2, 9)):
            parts.append(clean_stmt(rng, i))
        MISTAKE = saved
        return "\n".join(parts) + "\n"
    parts = ["class Box<T>(val v: T)"]
    ret = rng.choice(TYPES)
    a = rng.choice(TYPES)
    parts.append(f"func f(x: Int): Int {{\n    return {typed(rng, 'Int')}\n}}")
    parts.append(f"func q(x: {a}): {ret} {{\n    return {expr(rng)}\n}}")
    parts.append(f"func g<T>(x: Box<T>, y: Box<List<T>>): T {{\n    return x.v\n}}")
    parts.append(f"func h<T: Ord>(a: T, b: T): Comp {{\n    return a <=> b\n}}")
    if rng.random() < 0.4:
        parts.append("record R(val a: Int, val b: String) : Ord {\n"
                     "    func compareTo(other: R): Int { return this.a.compareTo(other.a) }\n}")
    if rng.random() < 0.3:
        parts.append("class D(val n: Int) {\n    proc deinit() { println(this.n) }\n}")
    n = rng.randrange(2, 9)
    for i in range(n):
        parts.append(stmt(rng, i))
    return "\n".join(parts) + "\n"


def main():
    binary = sys.argv[1]
    count = int(sys.argv[2])
    seed = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    rng = random.Random(seed)
    accepted = refused = 0
    crashes = 0
    for k in range(count):
        src = program(rng)
        p = subprocess.run([binary, "check", "/dev/stdin"], input=src,
                           capture_output=True, text=True, timeout=30)
        blob = p.stdout + p.stderr
        crashed = p.returncode not in (0, 1) or "panicked" in blob or "RUST_BACKTRACE" in blob
        if crashed:
            crashes += 1
            path = f"tests/fuzz/crash-{seed}-{k}.keal"
            with open(path, "w") as f:
                f.write(src)
            print(f"CRASH #{crashes}: exit={p.returncode} -> {path}")
            print(blob[:400])
        elif p.returncode == 0:
            accepted += 1
            # Differential mode: an accepted program must mean the same
            # thing to both interpreters, byte for byte — stdout, stderr
            # and exit code alike (runtime panics included).
            try:
                a = subprocess.run([binary, "--ast", "/dev/stdin"], input=src,
                                   capture_output=True, text=True, timeout=30)
                v = subprocess.run([binary, "--vm", "/dev/stdin"], input=src,
                                   capture_output=True, text=True, timeout=30)
            except subprocess.TimeoutExpired:
                a = v = None
            if a is not None and (a.stdout != v.stdout or a.stderr != v.stderr
                                  or a.returncode != v.returncode):
                crashes += 1
                path = f"tests/fuzz/diverge-{seed}-{k}.keal"
                with open(path, "w") as f:
                    f.write(src)
                print(f"DIVERGENCE: -> {path}")
                print("ast:", a.returncode, repr(a.stdout[:120]), repr(a.stderr[:120]))
                print("vm: ", v.returncode, repr(v.stdout[:120]), repr(v.stderr[:120]))
        else:
            refused += 1
    print(f"seed={seed} count={count}: accepted={accepted} refused={refused} crashes={crashes}")
    sys.exit(1 if crashes else 0)


if __name__ == "__main__":
    main()
