"""Count the `.keal` files across this owner's repositories and rewrite the
badge band at the top of README.md.

    python3 ci/count-keal-files.py [--check]

The band also carries the version, and takes it from Cargo.toml rather than
from a number typed into the README. A version written by hand in a second
place is a copy that drifts, and this repository has already been bitten by
that shape: `site/build.py` still holds a literal `v1.2.0`, and the test that
checks the site against its generator cannot see it, because the generator
faithfully rewrites the same stale string.

Why 2000 colours the badge. Linguist admits a new language once an extension
has "at least 2000 files per extension indexed in the last year, excluding
forks", and asks separately that those files "show a reasonable distribution
across unique :user/:repo combinations". This script can measure the first
and not the second — everything it counts belongs to one owner — so the
threshold is a colour here and nothing more. The README states neither, and
should not: a reader wants to know how much Keal is written, not what the
number is being saved up for.

It counts what is IN the repositories, which is not what GitHub's code
search reports — on 2026-09-05 the trees held 346 files and search answered
287, because indexing lags. Search is the number Linguist actually reads, so
this one runs ahead of it; the badge links to the search so a reader can see
both rather than take either on trust.

Forks are excluded, as the criterion excludes them. A truncated tree is a
hard error rather than a low number, because the failure a counter must not
have is looking fine while undercounting.
"""

import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

OWNER = "geneacta"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
README = os.path.join(ROOT, "README.md")
EXT = ".keal"

# Linguist's threshold for an extension expected to occur more than once per
# repository. The badge is coloured by how close the count is to it.
TARGET = 2000

START = "<!-- keal-band:start -->"
END = "<!-- keal-band:end -->"
CARGO = os.path.join(ROOT, "Cargo.toml")


def api(path):
    req = urllib.request.Request(
        "https://api.github.com/" + path.lstrip("/"),
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "keal-count",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        sys.exit("GitHub said %s for %s\n%s" % (e.code, path, e.read()[:200]))


def repos():
    """Every non-fork public repository of the owner, newest page first.

    Discovered rather than listed, so a repository added later is counted
    without anyone remembering to edit this file — the failure mode of a
    hardcoded list is a count that is quietly too low.
    """
    out, page = [], 1
    while True:
        got = api("users/%s/repos?per_page=100&page=%d" % (OWNER, page))
        if not got:
            break
        out += [r for r in got if not r["fork"]]
        if len(got) < 100:
            break
        page += 1
    return out


def count(repo):
    tree = api("repos/%s/%s/git/trees/%s?recursive=1"
               % (OWNER, repo["name"], repo["default_branch"]))
    if tree.get("truncated"):
        sys.exit("the tree of %s came back truncated, so this count would be\n"
                 "too low with nothing to show for it. Count that repository\n"
                 "another way before trusting the badge." % repo["name"])
    return sum(1 for e in tree.get("tree", [])
               if e["type"] == "blob" and e["path"].endswith(EXT))


def version():
    m = re.search(r'^version\s*=\s*"([^"]+)"', open(CARGO, encoding="utf-8").read(), re.M)
    if not m:
        sys.exit("Cargo.toml has no version for the band to carry.")
    return m.group(1)


# What the band looks like the first time it is written, and never again.
DEFAULT_BAND = """<p align="right">
  <a href="https://github.com/%(owner)s/keal/releases"><img alt="version" src="https://img.shields.io/badge/version-%(version)s-blue?style=flat"></a>
  <a href="%(search)s"><img alt=".keal files" src="https://img.shields.io/badge/.keal%%20files-%(count)d-brightgreen?style=flat"></a>
</p>"""


def band(existing, n):
    """The band with today's two numbers in it, and nothing else touched.

    This script counts. It does not decide how the count is drawn — and it
    used to: it rebuilt the whole band from its own opinion of the style,
    the colour and the alignment. Tony changed those by hand in `47a94c3`
    (right rather than centre, `flat` rather than `flat-square`, no
    `labelColor`) and the next run put its own back, twice. A generator that
    overwrites a decision is a generator nobody can work around, and the
    person it fights is the one who owns the file.

    So the numbers are substituted into whatever is there. Anyone may restyle
    the badges, move them, or add a third; the count and the version stay
    true because that is the only thing this knows.
    """
    search = ("https://github.com/search?q=" +
              urllib.parse.quote("extension:keal user:" + OWNER) + "&type=code")
    if "img.shields.io" not in existing:
        return DEFAULT_BAND % {"owner": OWNER, "version": version(),
                               "search": search, "count": n}
    out = re.sub(r"(badge/version-)[^-?]*(-)", r"\g<1>%s\g<2>" % version(), existing, count=1)
    out = re.sub(r"(badge/\.keal%20files-)[^-?]*(-)", r"\g<1>%d\g<2>" % n, out, count=1)
    return out


def main():
    check = "--check" in sys.argv[1:]
    per = [(r["name"], count(r)) for r in repos()]
    per = sorted(((n, c) for n, c in per if c), key=lambda x: -x[1])
    total = sum(c for _, c in per)
    for name, c in per:
        print("  %-12s %4d" % (name, c))
    print("  %-12s %4d  (%d%% of Linguist's %d)"
          % ("total", total, round(total * 100 / TARGET), TARGET))

    text = open(README, encoding="utf-8").read()
    if START not in text or END not in text:
        sys.exit("README.md has no %s / %s markers to write between." % (START, END))
    here = re.search(re.escape(START) + r"(.*?)" + re.escape(END), text, re.S)
    new = text[: here.start()] + START + "\n" + band(here.group(1).strip(), total).strip() \
        + "\n" + END + text[here.end() :]
    if new == text:
        print("band already current")
        return
    if check:
        sys.exit("the band in README.md is not what this script would write "
                 "(it should say %s / %d)" % (version(), total))
    open(README, "w", encoding="utf-8").write(new)
    print("band updated")


if __name__ == "__main__":
    main()
