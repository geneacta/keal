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
| `release.yml` | On a `v*` tag: builds the compiler for macOS (arm64, x86_64) and Linux, runs the suite and the bootstrap on each, and opens a **draft** release with the binaries attached | not yet |

`pages.yml` also needs the repository setting **Settings → Pages →
Source: GitHub Actions**, once.

Nothing else about the project depends on these files: `cargo test
--release` and `./bootstrap.sh` are the same commands the workflows run,
and they are what a contributor runs locally.
