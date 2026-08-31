# Continuous integration

Two workflows live here rather than in `.github/workflows/`, for one
practical reason: GitHub refuses a push that creates or edits a workflow
file unless the credentials carry the `workflow` scope, which the tooling
used to write this repository does not have. Copying a file through the
web UI takes a moment and needs no such token.

## Installing one

1. Open <https://github.com/geneacta/keal/new/main>
2. Name the file `.github/workflows/<name>.yml` — GitHub creates the
   directories as you type
3. Paste the contents of the file from this directory
4. **Commit changes**

## What is here

| file | what it does | installed? |
|---|---|---|
| `pages.yml` | Publishes `site/` to GitHub Pages on every push that touches it | yes |
| `release.yml` | On a `v*` tag: builds the compiler for macOS (arm64, x86_64) and Linux, runs the suite and the bootstrap on each, and opens a **draft** release with the binaries attached | yes |

`pages.yml` also needs the repository setting **Settings → Pages →
Source: GitHub Actions**, once.

Nothing else about the project depends on these files: `cargo test
--release` and `./bootstrap.sh` are the same commands the workflows run,
and they are what a contributor runs locally.

## Getting out of the copy-paste loop

Every fix to a workflow has to be pasted through the web UI because the
credentials used here lack the `workflow` scope. One command ends that:

```sh
gh auth refresh -s workflow
```

After it, `ci/*.yml` can be copied to `.github/workflows/` and pushed like
any other file, and this directory becomes a mirror rather than a staging
area.

## When a runner label goes away

GitHub retires runner images, and a job whose label no longer exists sits
in the queue forever rather than failing — the release never opens. If a
build leg is queued while the others have finished, that is the first thing
to check, and the fix is a new label in the matrix here, re-copied through
the web UI.

A workflow already installed does not have to be re-tagged to be re-run
after such a fix: **Actions → release → Run workflow** takes the tag as an
input, which is what that input is for.

## Windows, and where the time goes

Installing a toolchain through `winget`'s portable-package path can crawl:
on one machine, extraction ran at roughly nineteen files a minute with the
process burning seven seconds of CPU in twelve minutes — Defender's
real-time scanning, not the network. Extracting the same archive that
winget had already downloaded and verified took 59 seconds for 11,875
files. If a Windows setup step ever needs a toolchain, download and extract
the archive rather than going through winget, and budget accordingly.

A Windows machine also needs a C compiler that is *not* MSVC, for the
reason `docs/interop.md` gives, and a POSIX-threads MinGW-w64 if it is to
build actor programs. `rustup-init --default-host x86_64-pc-windows-gnu`
installs per-user with no administrator rights.
