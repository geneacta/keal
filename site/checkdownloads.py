#!/usr/bin/env python3
"""Every file the Kealler page offers, asked for rather than assumed.

    python3 site/checkdownloads.py

The page's download table is written from `KEALLER_BUILDS`, which is a list
of what *should* exist. A release can be cut with fewer — 0.1.1 was, because
the arm64 job failed and was allowed to fail so that the other three could be
published. The table did not know, and would have offered a file that is not
there.

A page that offers a file that is not there is worse than a page that says to
wait: the reader spends their trust before finding out. So this asks GitHub
for each one and exits non-zero on anything that is not 200.

It needs the network, which is why it is a separate script and not part of
`build.py` — a site that cannot be rebuilt on a train is a site nobody
rebuilds.
"""
import sys
import urllib.request
import urllib.error

sys.path.insert(0, "site")
import content as C

if not C.KEALLER_VERSION:
    print("no Kealler release named yet; nothing to check")
    sys.exit(0)

base = ("https://github.com/geneacta/keal/releases/download/kealler-v%s/%s.tar.gz")
bad = 0
for name, plat, arch in C.KEALLER_BUILDS:
    url = base % (C.KEALLER_VERSION, name)
    try:
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=20) as r:
            code = r.status
    except urllib.error.HTTPError as e:
        code = e.code
    except Exception as e:                      # no network, DNS, timeout
        print("could not ask about %s: %s" % (name, e))
        sys.exit(2)
    mark = "ok " if code == 200 else "GONE"
    print("%s %-28s %s  %s" % (mark, name, code, "%s %s" % (plat, arch)))
    if code != 200:
        bad += 1

if bad:
    print("\n%d of %d downloads the page offers do not exist." % (bad, len(C.KEALLER_BUILDS)))
    print("Either attach them to the release, or take the row out of KEALLER_BUILDS.")
    sys.exit(1)
print("\nevery download the page offers is there")
