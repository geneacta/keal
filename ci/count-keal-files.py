"""Count the `.keal` files across this owner's repositories and rewrite the
badge in README.md.

    python3 ci/count-keal-files.py [--check]

Why this number and not another. Linguist admits a new language once an
extension has "at least 2000 files per extension indexed in the last year,
excluding forks", and asks separately that those files "show a reasonable
distribution across unique :user/:repo combinations". This script measures
the first half and cannot measure the second: everything it counts belongs
to one owner, so a badge reading 2000 would satisfy the threshold and still
fail the distribution check. It is a progress bar for one of two conditions,
and the README says so beside it rather than letting the number imply more
than it is.

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

START = "<!-- keal-count:start -->"
END = "<!-- keal-count:end -->"


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


def badge(n):
    search = ("https://github.com/search?q=" +
              urllib.parse.quote("extension:keal user:" + OWNER) + "&type=code")
    return ('[![%s files](https://img.shields.io/badge/.keal_files-%d-%s'
            '?style=flat-square&labelColor=2b2b2b)](%s)'
            % (EXT, n, colour(n), search))


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
                 START + "\n" + badge(total) + "\n" + END,
                 text, count=1, flags=re.S)
    if new == text:
        print("badge already current")
        return
    if check:
        sys.exit("the badge in README.md is not what this script would write "
                 "(it should say %d)" % total)
    open(README, "w", encoding="utf-8").write(new)
    print("badge updated")


if __name__ == "__main__":
    main()
