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
