# The same four programs, in eight languages

`bench/` holds four Keal programs chosen for four different costs. This
directory holds those same four programs written once per language, so that
`keal build` can be measured against something other than itself.

The results are published at
[geneacta.github.io/keal/benchmark.html](https://geneacta.github.io/keal/benchmark.html),
one section per machine.

```
c/  cpp/  rust/  keal/  go/  java/  kotlin/  python/
run.py        builds them, checks them, times them, prints an entry
```

## Running it

```sh
python3 bench/ports/run.py            # uses target/release/keal
python3 bench/ports/run.py /path/to/keal
```

It builds every language it finds a toolchain for and names the ones it
skips — a machine without Kotlin still produces a usable entry. Then it
checks that all thirty-two programs print the same number, throws away one
warm-up round, and times the set twice under two experimental designs.

What it prints at the end is a Python block. Append it to `MACHINES` in
`site/bench.py`, fill in every `CHANGE-ME`, and rebuild the site with
`python3 site/build.py`. The page renders however many machines the list
holds and grows a cross-machine comparison as soon as there are two.

## Landing your entry when someone else lands theirs

Two machine entries are independent data, but they are not independent *text*:
they have the same keys in the same order, so git does not see two disjoint
insertions. It interleaves them line by line, and a conflicted `site/bench.py`
can hold one entry whose `key` came from one machine and whose toolchain
strings came from another. That chimera is well-formed Python, the generator
renders it without complaint, and no test in this repository would catch it —
the numbers would simply be wrong and attributed to a machine that never
produced them.

So do not resolve that file hunk by hunk. Take the other side whole and put
your block back:

```sh
git pull --rebase
# on a conflict in site/bench.py:
git checkout --ours site/bench.py     # during a rebase, --ours is the UPSTREAM
                                      # side, the opposite of what it means in
                                      # a merge
# paste your entry at the end of MACHINES again, then:
git add site/bench.py
```

`site/benchmark.html` and `site/fr/benchmark.html` will conflict too. Never
merge those: they are generated. Take either side, run `python3 site/build.py`
on the resolved `bench.py`, and `git add` what it writes.

Then `cargo test --release` before you continue the rebase — the site test
compares the committed pages against what the generator would write, so it
catches a page you forgot to regenerate. It will not catch a chimera entry.

## The rules that make a second machine worth having

**Do not change the programs, and do not change the sizes.** They were
raised from the defaults in `bench/` until the compiled languages cleared
the noise floor — at the original sizes, C's compute time on two of the four
was indistinguishable from process startup. Changing them locally makes your
numbers incomparable with everyone else's, which is the only thing a second
machine is for. If a size is wrong for your hardware, say so rather than
editing it: the fix has to happen for every machine at once.

**Absolute milliseconds do not travel; ratios mostly do.** The page never
puts two machines' raw times side by side. It compares them on the ratio to
C, and where two machines disagree about a ratio, that disagreement is the
result — it means the number was a property of one box rather than of the
language.

**Report what the harness reports.** If it says a configuration's run order
moved its numbers by more than that configuration's own spread, that is a
finding about your machine, not a reason to run again until it goes away.
The same is true of a wide spread: a figure whose median sits far above its
minimum is not ranked against a close neighbour, and the page prints the
spread so a reader can see which ones those are.

**A benchmark is an instrument.** Every check in `run.py` could come back
negative, and one of them did the first time it ran: the Kotlin hello-world
silently failed to build, so its startup baseline was a JVM refusing to
start in 2.4 ms, and subtracting that inflated every Kotlin figure by twenty
milliseconds. The harness now runs each hello-world and requires it to print
`hi` before its time is subtracted from anything.
