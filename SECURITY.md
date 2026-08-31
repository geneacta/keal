# Security Policy

## Supported versions

Keal is pre-1.0 and moves fast. Security fixes land on `main`, and the
version they ship in is the next release; there are no maintenance
branches for older versions yet.

| Version | Supported |
|---|---|
| 0.5.x | yes — fixes land on `main` |
| < 0.5 | no |

## The threat model, stated plainly

Before reporting, it helps to know what this project does and does not
claim, because a compiler's security boundary is not where people
usually assume it is.

**Compiling a Keal program is running its author's code.** A source
file may contain a `native """ ... """` block, which is C pasted
verbatim into the generated translation unit, and `extern func`, which
calls any symbol on the link line. Both are documented features, not
holes. `keal build` also invokes a C compiler and a linker with the
arguments it was given. **So: do not compile Keal source you would not
run.** A hosted service that compiles untrusted user input needs a
sandbox around the whole toolchain — that is the operator's boundary to
draw, and no compiler flag can draw it for you.

What Keal *does* claim, and what a report should be about:

* **A safe program must compile to a safe binary.** A program with no
  `native` block and no `extern` must never produce a use-after-free, a
  double free, an out-of-bounds access, or an uninitialised read in the
  generated C. If you have one, that is the most serious kind of bug
  this project can have — the whole memory model
  ([`docs/memory.md`](docs/memory.md)) rests on it.
* **The type checker must be sound where the native backend trusts it.**
  The C backend compiles what the checker accepted, and narrowing an
  `Any`, an actor message crossing threads, or a generic instantiation
  all trust that the checker was right. A program that type-checks and
  then confuses two types at run time is a security bug, not just a
  correctness one.
* **The compiler must not crash on malformed input.** A panic in the
  lexer, parser or checker on any input file is a bug we want (the
  differential fuzzer in `tests/fuzz/` exists for exactly this). It is
  reportable here even though the impact is a crash rather than a
  compromise.
* **The toolchain must not read or write outside what it was asked to.**
  `import` resolves relative to the importing file; `keal jbind` writes
  its cache under `.jbind/`. A path in a source file that escapes those
  is a bug.

Out of scope, because they are the documented design: `native` blocks
executing arbitrary C; `extern` calling arbitrary symbols; a program
crashing itself with `throw`; `unsafe` behaviour reached through a C
library the program chose to link.

## Reporting a vulnerability

**Please do not open a public issue for a security bug.**

Use GitHub's private reporting — the **Security** tab on
[github.com/geneacta/keal](https://github.com/geneacta/keal), then
"Report a vulnerability" — or write to **contact@geneacta.com** if you
would rather use email.

Please include, as far as you can:

* the smallest Keal program that shows the problem, and the command line
  you ran (`keal build f.keal`, `keal --vm f.keal`, …);
* what you expected and what happened instead, including the generated
  C (`keal emit-c f.keal`) when the report is about a compiled binary;
* which engine or engines it reproduces on — the tree-walker (`--ast`),
  the bytecode VM (the default), the native backend, or the self-hosted
  compiler in `selfhost/`.

You will get an acknowledgement within a week. This is a small project,
so what happens next is written honestly rather than promised in
service-level terms: we confirm the report, agree a fix and a timeline
with you, and credit you in the commit and release notes unless you ask
us not to. If a report turns out to be a plain bug rather than a
vulnerability we will say so and move it to a public issue, with your
agreement.

Thank you for taking the time. A report that makes a compiler safer
makes every program it compiles safer.
