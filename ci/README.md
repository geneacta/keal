# Continuous integration

The workflows themselves live in `.github/workflows/`, which is where
GitHub reads them. This directory holds what they need and what a person
needs to know about them.

Until 2026-09-01 the two workflows lived HERE as well, because the
credentials used to write this repository lacked the `workflow` scope and
GitHub refuses a push that creates or edits a workflow file without it. So
`ci/` was a staging area and every fix had to be pasted through the web UI.

That arrangement produced exactly the defect it was shaped to produce.
`ci/release.yml` said `draft: false` from commit `d78bf7d`; the file GitHub
actually ran still said `draft: true`, because the paste never happened. For
weeks every release was opened by hand and nobody could see why — the fix was
committed, reviewed and true, and it was not the file being executed.

Two copies that must agree, and nothing checking that they do, is the shape
this project has spent a week learning to recognise. The scope exists now, so
there is one copy.

## What runs

| workflow | when | what |
|---|---|---|
| `check.yml` | every push to `main`, every pull request | the suite and the bootstrap, on **Linux** |
| `smoke.yml` | a release is published, or by hand with a tag | downloads the PUBLISHED archive on each of the four platforms, unpacks it, and asks that binary to run a program and then compile one |
| `pages.yml` | every push touching `site/` | publishes to GitHub Pages |
| `release.yml` | a `v*` tag, or **Actions → release → Run workflow** with a tag | builds for macOS (arm64, x86_64), Linux and Windows, runs the suite and the bootstrap on each, and opens a release with the binaries attached |

`pages.yml` also needs the repository setting **Settings → Pages →
Source: GitHub Actions**, once.

### Why `check.yml` is Linux

The four-platform suite runs only on a tag, so between releases the only
machines that ever ran it were the ones a person happened to be sitting at.
On 2026-09-01 that meant `keal build` had been broken on Linux for a day —
`-std=c11` makes glibc withhold the POSIX half of `<time.h>` while Apple's
headers declare it regardless — and the release was the first thing to ask
Linux anything. Twelve tests failed at once, on a tag.

Linux is the leg to run per push because it is the one nobody develops on:
macOS and Windows each have a person watching them. It is also the strictest
of the three about what a header declares, which is the failure it just
caught.

### Why `smoke.yml` exists

The release workflow tests the compiler it built and then packages it. It
never opens the tarball. So the archive a person downloads was an output with
no consumer — and when 1.1.0 went out, nobody involved had a Linux machine
and the Linux build had been executed by exactly one process: a CI runner
that then deleted itself.

It cannot catch everything, and it is not meant to: the suite already asked
the compiler every question worth asking. What this catches is the class
where the ARCHIVE is the broken part rather than the compiler — a missing or
misnamed binary, a wrong architecture, a dynamic-link failure, a permission
bit lost in the tarball. `ci/smoke.keal` is what it runs, and it asserts
rather than prints, so a sound release produces `ok` and nothing else.

### A prepared machine and a bare one are different claims

Every Windows measurement this project had for its first two days on that
platform came from a machine a person had equipped: a MinGW they chose, a JDK
they chose, a `python3` shim they created. That is a real and useful thing to
have, and it is not the same claim as "this works on Windows" — it is "this
works on a Windows somebody set up for it".

`smoke.yml` runs the published archive on a bare `windows-latest` that has
none of that until the workflow puts it there. Both readings are worth
having; the mistake is letting one stand in for the other, which is easy
because both are green ticks on the same platform name.

### What a check may cost

A check that takes two minutes gets re-run when somebody is unsure. A check
that takes twenty gets argued with, then skipped, then removed. That is a
constraint on what these workflows may do, not a nicety: `check.yml` is
Linux-only for that reason as much as for the coverage one.

The Windows smoke leg costs about 125 seconds, essentially all of it the
toolchain install — no other leg installs one. That is on the right side of
the line, and it is worth glancing at again if the runner image changes,
because the whole cost is one step and nothing else in that leg is slow. The
same install through `winget`'s portable path took forty minutes on a real
machine with Defender running, which is the wrong side of it.

## When a runner label goes away

GitHub retires runner images, and a job whose label no longer exists sits in
the queue forever rather than failing — the release never opens. If a build
leg is queued while the others have finished, that is the first thing to
check.

A workflow already installed does not have to be re-tagged to be re-run after
such a fix: **Actions → release → Run workflow** takes the tag as an input,
which is what that input is for.

## Windows, and where the time goes

`ci/windows-toolchain.sh` installs the compiler the Windows leg needs and
asserts what it installed rather than trusting it. It lives here rather than
inside the workflow so that the thing most likely to need changing can change
without touching a workflow file.

A Windows runner has no `cc`, and the compiler this runtime needs is **not**
MSVC: overflow is checked with GCC and Clang builtins. Nor is any compiler
enough — an LLVM `clang` on a Windows runner targets MSVC, rejects
`-pthread`, and would satisfy a naive "does a compiler answer" check while
building nothing. Building actor programs also needs a **POSIX-threads**
MinGW-w64; the `win32` and `mcf` flavours ship no `pthread.h`.

Installing a toolchain through `winget`'s portable-package path can crawl: on
one machine, extraction ran at roughly nineteen files a minute with the
process burning seven seconds of CPU in twelve minutes — Defender's real-time
scanning, not the network. Extracting the same archive winget had already
downloaded took 59 seconds for 11,875 files. Download and extract rather than
going through winget, and budget accordingly.

`rustup-init --default-host x86_64-pc-windows-gnu` installs per-user with no
administrator rights.

## Nothing here is load-bearing for a contributor

`cargo test --release` and `./bootstrap.sh` are the same commands these
workflows run, and they are what to run locally.
