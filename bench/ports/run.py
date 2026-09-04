"""Run the cross-language benchmark on this machine and print one machine
entry for `site/bench.py`.

    python3 bench/ports/run.py [path/to/keal]

What it does, in the order it does it, because the order is the point:

 1. Builds every language it finds a toolchain for. A missing toolchain is
    skipped and named, never a failure — a machine without Kotlin still
    produces a usable entry.
 2. Runs every program once and compares its last line against the expected
    output. A program that prints a different number is not timed at all.
 3. Runs every configuration once and throws the result away, so the page
    cache is full before any clock starts and nothing pays for being first.
 4. Times the whole set TWICE, under two designs: blocked, where a
    configuration's replicates run consecutively, and shuffled, where the
    whole sequence is drawn at random. Comparing the two is what says
    whether run order is biasing the numbers.
 5. Reports each configuration's own spread, and prints the entry.

The reported figure is the MINIMUM of the pooled runs minus that runtime's
own hello-world time. The minimum rather than the median because a machine
under interference produces a long right tail and no left one; the startup
subtraction so a JVM's boot is not charged to its arithmetic.

------------------------------------------------------------------------
ADDING YOUR MACHINE

Run this file, then paste the printed block into the `MACHINES` list in
`site/bench.py` — append it, do not replace anything — and rebuild the site
with `python3 site/build.py`. The page renders however many machines the
list holds, and adds a cross-machine comparison as soon as there are two.

Do not change the programs in this directory, and do not change the sizes.
They were raised from the defaults in `bench/` until the compiled languages
cleared the noise floor on the first machine; changing them makes your
numbers incomparable with everyone else's, which is the only thing a second
machine is for. If a size is wrong for your hardware — a machine ten times
faster would put C back under the floor — say so rather than editing it
locally, because the fix has to happen for every machine at once.

Absolute milliseconds do not carry across machines. The ratio to C does,
which is why the page compares machines on ratios and never on raw times.
"""

import json
import os
import random
import shutil
import statistics as st
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
WIN = os.name == "nt"
EXE = ".exe" if WIN else ""
OUT = os.path.join(HERE, "_build")

KEAL = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    ROOT, "target", "release", "keal" + EXE)

REPLICATES = 9
STARTUP_RUNS = 15

# key, display name, expected last line of output
PROGRAMS = [
    ("fib",     "fib(35)", "9227465"),
    ("loops",   "loops",   "299999995"),
    ("objects", "objects", "342432475"),
    ("lists",   "lists",   "748500000"),
]
JAVA_CLASS = {"fib": "Fib", "loops": "Loops", "objects": "Objects", "lists": "Lists"}

LANGS = ["C", "C++", "Rust", "Keal", "Go", "Java", "Kotlin", "Python"]


def which(*names):
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", **kw)


def version(cmd, pick=lambda s: s.splitlines()[0] if s.splitlines() else "?"):
    try:
        r = run(cmd)
        return pick((r.stdout or r.stderr).strip())
    except Exception:
        return "?"


# ---------------------------------------------------------------- building

class Lang:
    """One language: how to build its four programs, and how to run one."""

    def __init__(self, name, tool, build, cmd, ver, flags):
        self.name, self.tool = name, tool
        self.build, self.cmd, self.ver, self.flags = build, cmd, ver, flags
        self.ok = tool is not None
        self.why = "" if self.ok else "no toolchain"


def build_all():
    os.makedirs(OUT, exist_ok=True)
    langs = {}

    cc = which("gcc", "cc", "clang")
    cxx = which("g++", "c++", "clang++")
    rustc = which("rustc")
    go = which("go")
    javac, java = which("javac"), which("java")
    kotlinc = which("kotlinc", "kotlinc.bat")
    py = sys.executable
    keal = KEAL if os.path.exists(KEAL) else None

    def o(name):
        return os.path.join(OUT, name + EXE)

    langs["C"] = Lang("C", cc,
        lambda k: run([cc, "-O2", "-std=c11", "-o", o("c_" + k),
                       os.path.join(HERE, "c", k + ".c")]),
        lambda k: [o("c_" + k)],
        lambda: version([cc, "--version"]), "-O2 -std=c11")

    langs["C++"] = Lang("C++", cxx,
        lambda k: run([cxx, "-O2", "-std=c++17", "-o", o("cpp_" + k),
                       os.path.join(HERE, "cpp", k + ".cpp")]),
        lambda k: [o("cpp_" + k)],
        lambda: version([cxx, "--version"]), "-O2 -std=c++17")

    langs["Rust"] = Lang("Rust", rustc,
        lambda k: run([rustc, "-C", "opt-level=2", "-o", o("rs_" + k),
                       os.path.join(HERE, "rust", k + ".rs")]),
        lambda k: [o("rs_" + k)],
        lambda: version([rustc, "--version"]), "-C opt-level=2")

    # `keal build` names its output from the source's stem and writes it into
    # the CURRENT directory, not beside the source (src/nativebuild.rs). So the
    # `cwd` below and the path on the line after it have to agree; they do only
    # because both name the source directory. Change one and the harness looks
    # for a binary that was written somewhere else.
    langs["Keal"] = Lang("Keal", keal,
        lambda k: run([keal, "build", os.path.join(HERE, "keal", k + ".keal")],
                      cwd=os.path.join(HERE, "keal")),
        lambda k: [os.path.join(HERE, "keal", k + EXE)],
        lambda: version([keal, "version"]), "keal build")

    def go_build(k):
        d = os.path.join(HERE, "go")
        if not os.path.exists(os.path.join(d, "go.mod")):
            run([go, "mod", "init", "kealbench"], cwd=d)
        return run([go, "build", "-o", o("go_" + k), "./" + k], cwd=d)

    langs["Go"] = Lang("Go", go, go_build,
        lambda k: [o("go_" + k)],
        lambda: version([go, "version"]), "go build")

    langs["Java"] = Lang("Java", javac and java,
        lambda k: run([javac, "-d", os.path.join(OUT, "javacls"),
                       os.path.join(HERE, "java", JAVA_CLASS[k] + ".java")]),
        lambda k: [java, "-cp", os.path.join(OUT, "javacls"), JAVA_CLASS[k]],
        lambda: version([java, "-version"]), "javac, default JVM")

    def kotlin_version():
        """Kotlin's compiler, and the JVM that actually ran the jars.

        `kotlinc -version` names the JVM the COMPILER runs on. Where kotlinc
        ships or picks its own JDK that is not the JVM the benchmark used —
        the timed runs go through `java -jar`, which is whatever `java` is on
        the PATH. Reporting only the first names a runtime that never executed
        a program, and the macOS bench declared JRE 26 for rows that had run
        on 23. Both are printed, so the column cannot say one and mean the
        other.
        """
        kc = version([kotlinc, "-version"],
                     lambda t: t.replace("info: ", "").splitlines()[0])
        return "%s; jars run on %s" % (kc, version([java, "-version"]))

    langs["Kotlin"] = Lang("Kotlin", kotlinc if (kotlinc and java) else None,
        lambda k: run([kotlinc, os.path.join(HERE, "kotlin", k + ".kt"),
                       "-include-runtime", "-d", os.path.join(OUT, "kt_" + k + ".jar")]),
        lambda k: [java, "-jar", os.path.join(OUT, "kt_" + k + ".jar")],
        kotlin_version,
        "-include-runtime, jars run on the PATH java")

    langs["Python"] = Lang("Python", py, lambda k: None,
        lambda k: [py, os.path.join(HERE, "python", k + ".py")],
        lambda: "CPython " + ".".join(str(x) for x in sys.version_info[:3]), "stock build")

    print("building")
    for name in LANGS:
        L = langs[name]
        if not L.ok:
            print("  %-7s skipped — %s" % (name, L.why))
            continue
        for key, _, _ in PROGRAMS:
            try:
                r = L.build(key)
            except Exception as e:
                L.ok, L.why = False, str(e)[:60]
                break
            if r is not None and r.returncode != 0:
                L.ok, L.why = False, (r.stderr or r.stdout).strip().splitlines()[:1]
                L.why = L.why[0][:60] if L.why else "build failed"
                break
        print("  %-7s %s" % (name, "ok" if L.ok else "FAILED — " + str(L.why)))
    return langs


# ---------------------------------------------------------------- hello

def hello_cmds(langs):
    """A hello-world per language, for the startup baseline.

    Every one of these is RUN and checked to print `hi` before it is
    accepted. A baseline that silently failed to build would be a couple of
    milliseconds of a runtime refusing to start, and subtracting that from a
    real measurement inflates the language it belongs to — which is exactly
    what happened to Kotlin the first time this was run, and was only caught
    because a JVM cannot start in 2.4 ms. A hello that does not print `hi`
    stops the run rather than quietly biasing it.
    """
    d = os.path.join(OUT, "hello")
    os.makedirs(d, exist_ok=True)
    src = {
        "C":      ("h.c",    '#include <stdio.h>\nint main(void){puts("hi");return 0;}\n'),
        "C++":    ("h.cpp",  '#include <cstdio>\nint main(){std::puts("hi");return 0;}\n'),
        "Rust":   ("h.rs",   'fn main(){println!("hi");}\n'),
        "Keal":   ("h.keal", 'println("hi")\n'),
        "Go":     ("main.go", 'package main\nimport "fmt"\nfunc main(){fmt.Println("hi")}\n'),
        "Java":   ("H.java", 'public class H{public static void main(String[] a){System.out.println("hi");}}\n'),
        "Kotlin": ("h.kt",   'fun main(){println("hi")}\n'),
        "Python": ("h.py",   'print("hi")\n'),
    }
    out = {}
    for name, (fn, text) in src.items():
        L = langs[name]
        if not L.ok:
            continue
        sub = d if name != "Go" else os.path.join(d, "go")
        os.makedirs(sub, exist_ok=True)
        p = os.path.join(sub, fn)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        try:
            if name == "C":
                run([L.tool, "-O2", "-std=c11", "-o", os.path.join(d, "h_c" + EXE), p])
                out[name] = [os.path.join(d, "h_c" + EXE)]
            elif name == "C++":
                run([L.tool, "-O2", "-std=c++17", "-o", os.path.join(d, "h_cpp" + EXE), p])
                out[name] = [os.path.join(d, "h_cpp" + EXE)]
            elif name == "Rust":
                run([L.tool, "-C", "opt-level=2", "-o", os.path.join(d, "h_rs" + EXE), p])
                out[name] = [os.path.join(d, "h_rs" + EXE)]
            elif name == "Keal":
                run([KEAL, "build", p], cwd=sub)
                out[name] = [os.path.join(sub, "h" + EXE)]
            elif name == "Go":
                go = which("go")
                if not os.path.exists(os.path.join(sub, "go.mod")):
                    run([go, "mod", "init", "hello"], cwd=sub)
                run([go, "build", "-o", os.path.join(d, "h_go" + EXE), "."], cwd=sub)
                out[name] = [os.path.join(d, "h_go" + EXE)]
            elif name == "Java":
                run([which("javac"), "-d", os.path.join(d, "hcls"), p])
                out[name] = [which("java"), "-cp", os.path.join(d, "hcls"), "H"]
            elif name == "Kotlin":
                run([which("kotlinc", "kotlinc.bat"), p, "-include-runtime",
                     "-d", os.path.join(d, "h_kt.jar")])
                out[name] = [which("java"), "-jar", os.path.join(d, "h_kt.jar")]
            elif name == "Python":
                out[name] = [sys.executable, p]
        except Exception as e:
            sys.exit("could not build the %s hello-world (%s); its startup baseline\n"
                     "would be missing, and every %s figure would be inflated by\n"
                     "whatever the failure costs." % (name, str(e)[:60], name))
    for name, cmd in sorted(out.items()):
        r = run(cmd)
        if "hi" not in (r.stdout or ""):
            sys.exit("the %s hello-world does not print `hi`: %r\n"
                     "Its startup baseline would be a runtime refusing to start,\n"
                     "and subtracting that inflates every %s figure. Fix the\n"
                     "toolchain or drop the language; do not measure around it."
                     % (name, ((r.stdout or "") + (r.stderr or "")).strip()[:80], name))
    missing = [n for n in langs if langs[n].ok and n not in out]
    if missing:
        sys.exit("no hello-world for: %s" % ", ".join(missing))
    return out


# ---------------------------------------------------------------- timing

def once(cmd):
    t0 = time.perf_counter()
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return (time.perf_counter() - t0) * 1000.0


def main():
    langs = build_all()
    live = [n for n in LANGS if langs[n].ok]
    if "C" not in live:
        sys.exit("C is the baseline every ratio is taken against; it must build.")

    cfgs = [(n, k) for n in live for k, _, _ in PROGRAMS]

    print("\nchecking that every program prints the same thing")
    bad = []
    for n, k in cfgs:
        exp = dict((a, c) for a, _, c in PROGRAMS)[k]
        r = run(langs[n].cmd(k))
        got = (r.stdout or "").strip().splitlines()
        got = got[-1] if got else "<empty>"
        if got != exp:
            bad.append("%s/%s printed %s, expected %s" % (n, k, got, exp))
    if bad:
        print("\n".join("  " + b for b in bad))
        sys.exit("outputs disagree; nothing was timed")
    print("  %d of %d agree" % (len(cfgs), len(cfgs)))

    hello = hello_cmds(langs)
    print("\nwarming the page cache (one round, discarded)")
    for n, k in cfgs:
        once(langs[n].cmd(k))
    for c in hello.values():
        once(c)

    blocked, shuffled = [], []
    for c in cfgs:
        blocked.extend([c] * REPLICATES)
        shuffled.extend([c] * REPLICATES)
    random.shuffle(shuffled)

    def measure(jobs, label):
        res = {}
        for i, c in enumerate(jobs):
            res.setdefault(c, []).append(once(langs[c[0]].cmd(c[1])))
            if i % 40 == 0:
                print("    %d/%d" % (i, len(jobs)), flush=True)
        print("  %s: %d runs" % (label, len(jobs)))
        return res

    print("\ntiming, design A (blocked)")
    A = measure(blocked, "blocked")
    print("\ntiming, design B (shuffled)")
    B = measure(shuffled, "shuffled")

    start = {}
    sj = [n for n in hello for _ in range(STARTUP_RUNS)]
    random.shuffle(sj)
    for n in sj:
        start.setdefault(n, []).append(once(hello[n]))
    start = {n: min(v) for n, v in start.items()}

    print("\n--- order control ---------------------------------------------")
    print("Does the gap between the two designs exceed a configuration's own")
    print("spread within one design? If not, run order is not biasing anything.")
    flagged = 0
    for c in cfgs:
        a, b = A[c], B[c]
        ra = (max(a) - min(a)) / st.median(a) * 100
        rb = (max(b) - min(b)) / st.median(b) * 100
        gap = abs(st.median(b) - st.median(a)) / st.median(a) * 100
        if gap > max(ra, rb):
            flagged += 1
            print("  ORDER EFFECT  %s/%s  gap %.0f%% vs spread %.0f%%/%.0f%%"
                  % (c[0], c[1], gap, ra, rb))
    print("  %d of %d configurations show a gap above their own noise" % (flagged, len(cfgs)))
    if flagged:
        print("  -> do NOT paste this entry; say so instead, it is a real finding")

    ms, spread = {}, {}
    for n in live:
        pooled = [A[(n, k)] + B[(n, k)] for k, _, _ in PROGRAMS]
        s = start.get(n, 0.0)
        ms[n] = [round(min(p) - s, 1) for p in pooled]
        spread[n] = [round((st.median(p) - min(p)) / min(p) * 100) for p in pooled]

    entry = {
        "key": "CHANGE-ME",
        "name_en": "CHANGE-ME", "name_fr": "CHANGE-ME",
        "cpu_en": "CHANGE-ME", "cpu_fr": "CHANGE-ME",
        # `nt win32` is not something a reader learns anything from, so this
        # asks to be replaced like the other CHANGE-MEs rather than looking
        # filled in. Name the distribution or the OS version.
        "os": "CHANGE-ME (the harness only saw: %s %s)" % (os.name, sys.platform),
        "date": time.strftime("%Y-%m-%d"),
        "keal": version([KEAL, "version"]) if os.path.exists(KEAL) else "?",
        "runs": REPLICATES * 2,
        "order_effects": flagged,
        "toolchains": [[n, langs[n].ver(), langs[n].flags] for n in live],
        "startup": {n: round(start.get(n, 0.0), 1) for n in live},
        "ms": ms,
        "spread": spread,
    }
    print("\n--- paste this into MACHINES in site/bench.py ------------------")
    print(json.dumps(entry, indent=8, ensure_ascii=False)
          .replace("true", "True").replace("false", "False").replace("null", "None"))
    print("\nFill in every CHANGE-ME, and describe the machine the way a reader")
    print("who has never seen it would need: cores, architecture, and whether")
    print("it is bare metal or a guest.")
    print("\nIf any version above was arranged for this run rather than being what")
    print("the machine gives by default — a JDK picked with JAVA_HOME, an")
    print("interpreter chosen to launch this file — add a `note_en` and `note_fr`")
    print("saying so. Someone reproducing it with the machine's own defaults would")
    print("otherwise get different numbers and no way to know why.")
    print("\nAnd note that pulling a newer run.py while this one is running changes")
    print("nothing about what it prints: Python has already read the file.")


if __name__ == "__main__":
    main()
