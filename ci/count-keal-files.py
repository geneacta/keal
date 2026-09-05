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


def colour(n):
    if n >= TARGET:
        return "brightgreen"
    if n >= TARGET // 2:
        return "green"
    if n >= TARGET // 10:
        return "blue"
    return "lightgrey"


def version():
    m = re.search(r'^version\s*=\s*"([^"]+)"', open(CARGO, encoding="utf-8").read(), re.M)
    if not m:
        sys.exit("Cargo.toml has no version for the band to carry.")
    return m.group(1)


def shield(label, message, col, href, alt):
    return ('  <a href="%s"><img alt="%s" src="https://img.shields.io/badge/'
            '%s-%s-%s?style=flat-square&labelColor=2b2b2b"></a>'
            % (href, alt, urllib.parse.quote(label), urllib.parse.quote(message), col))


def band(n):
    search = ("https://github.com/search?q=" +
              urllib.parse.quote("extension:keal user:" + OWNER) + "&type=code")
    return "\n".join([
        "<p align=\"center\">",
        shield("version", version(), "blue",
               "https://github.com/%s/keal/releases" % OWNER, "version"),
        shield(".keal files", str(n), colour(n), search, ".keal files"),
        "</p>",
    ])


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
    new = re.sub(re.escape(START) + r".*?" + re.escape(END),
                 START + "\n" + band(total) + "\n" + END,
                 text, count=1, flags=re.S)
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
