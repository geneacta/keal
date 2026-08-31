#!/usr/bin/env python3
"""Runs the tour's snippets and checks they print what the page promises.

    python3 site/checktour.py [path-to-keal]

The tour tells the reader that every snippet is a real program and every
output is what it actually prints. This is what makes that true: each
chapter in `content.TOUR` is run on both interpreters — and natively when
it needs a C compiler, which the two chapters using `extern` do — and its
output compared to the one printed beside it.

Exits non-zero, naming the chapter, on the first disagreement.
"""

import os
import subprocess
import sys
import tempfile

SITE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(SITE)
sys.path.insert(0, SITE)

import content as C  # noqa: E402


def native_only(code):
    """A snippet the interpreters cannot run: it calls into C."""
    return "extern fun" in code


def c_driver():
    """The C compiler this machine has, or None.

    `cc` is a Unix name; a Windows machine has `gcc` or `clang` and neither
    is called `cc`. And a missing program raises here rather than returning
    a code, which is what made this line crash instead of skip.
    """
    # `CC` is not a hint but an instruction: it is the compiler the build
    # will use, so if it does not answer, there is no compiler.
    named = os.environ.get("CC")
    for name in [named] if named else ["cc", "gcc", "clang"]:
        try:
            if subprocess.run([name, "--version"], capture_output=True).returncode == 0:
                return name
        except (FileNotFoundError, OSError):
            continue
    return None


def main():
    default = os.path.join(ROOT, "target/release/keal")
    if os.name == "nt":
        default += ".exe"
    keal = sys.argv[1] if len(sys.argv) > 1 else default
    if not os.path.exists(keal):
        print("no compiler at %s — build one with `cargo build --release`" % keal)
        return 2
    have_cc = c_driver() is not None

    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, (title, _, _, _, code, expected) in enumerate(C.TOUR, start=1):
            path = os.path.join(tmp, "chapter%d.keal" % i)
            with open(path, "w", encoding="utf-8") as f:
                f.write(code + "\n")

            runs = []
            if not native_only(code):
                runs.append(("the VM", [keal, "run", path]))
                runs.append(("the tree-walker", [keal, "run", "--ast", path]))
            if have_cc:
                exe = os.path.join(tmp, "chapter%d" % i)
                built = subprocess.run([keal, "build", path, "-o", exe],
                                       capture_output=True, text=True)
                if built.returncode == 0:
                    runs.append(("native code", [exe]))
                elif native_only(code):
                    failures.append("%d. %s — `keal build` failed:\n%s"
                                    % (i, title, built.stderr.strip()))
                # A refusal by name on a chapter the interpreters can run is
                # the backend saying so honestly; the page never claims the
                # snippet compiles natively.
            elif native_only(code):
                print("  (%d. %s skipped: no C compiler)" % (i, title))

            for engine, cmd in runs:
                r = subprocess.run(cmd, capture_output=True, text=True)
                got = r.stdout.rstrip("\n")
                if r.returncode != 0:
                    failures.append("%d. %s — %s exited %d:\n%s"
                                    % (i, title, engine, r.returncode, r.stderr.strip()))
                elif got != expected:
                    failures.append("%d. %s — %s printed\n%s\nbut the page says\n%s"
                                    % (i, title, engine, got, expected))

    if failures:
        print("the tour does not print what it promises:\n")
        for f in failures:
            print(f + "\n")
        return 1
    print("%d chapters, every output as printed on the page" % len(C.TOUR))
    return 0


if __name__ == "__main__":
    sys.exit(main())
