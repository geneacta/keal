#!/usr/bin/env python3
"""Builds the whole Keal site, in English and in French.

    python3 site/build.py [path-to-keal]

Every page is generated: the landing page, the tour, the reference
documents (converted from `docs/*.md`), the standard library (from
`keal doc`), and one "coming from X" page per language. English lands in
`site/`, French in `site/fr/`, and each page links to its counterpart, so
neither language is a second-class citizen with half the pages missing.

No dependencies, no build step beyond this file: the output is plain HTML
GitHub Pages can serve as it stands.
"""

import html
import os
import re
import subprocess
import sys

# Every file this reads and writes is UTF-8, and every line it writes ends
# in `\n`, and both have to be said out loud. Python opens text files in the
# machine's locale codepage — on a French Windows that is cp1252, where the
# first `→` in a page raises, after the file has been truncated for writing,
# which turns a failed build into a deleted page. And text mode there
# rewrites every `\n` as `\r\n`, which makes a build report fifty files
# changed and hides the one that really did.
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SITE = os.path.join(ROOT, "site")
# `keal.exe` on Windows, and nothing there is called `keal`. The same line
# crashed `checktour.py` before a test caught it; nothing tests this file,
# so it is fixed here on the strength of that one rather than its own.
_DEFAULT_KEAL = os.path.join(ROOT, "target/release/keal")
if os.name == "nt":
    _DEFAULT_KEAL += ".exe"
KEAL = sys.argv[1] if len(sys.argv) > 1 else _DEFAULT_KEAL

# ---- a small markdown converter -----------------------------------------
# Enough of markdown for the documents this repository actually writes:
# headings, fenced code, tables, lists, quotes, rules, links, emphasis.

INLINE_CODE = re.compile(r"`([^`]+)`")
BOLD = re.compile(r"\*\*([^*]+)\*\*")
ITALIC = re.compile(r"(?<![*\w])\*([^*\n]+)\*(?!\*)")
LINK = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")


def slug(text):
    s = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return s or "section"


def fix_href(href):
    """Repoint a repository-relative link at the page the site generates."""
    if href.startswith(("http://", "https://", "#", "mailto:")):
        return href
    anchor = ""
    if "#" in href:
        href, anchor = href.split("#", 1)
        anchor = "#" + anchor
    base = os.path.basename(href)
    if base.endswith(".md"):
        name = base[:-3]
        mapped = {
            "language": "language.html",
            "types": "types.html",
            "memory": "memory.html",
            "threads": "threads.html",
            "interop": "interop.html",
            "drop": "deinit.html",
            "packages": "packages.html",
            "README": "index.html",
            "TUTORIAL": "tour.html",
            "CONTRIBUTING": "contributing.html",
        }
        if name in mapped:
            return mapped[name] + anchor
        # An unmapped `.md` used to become a link to the docs index. That is
        # never a broken link and never the right one: `packages.md` had a
        # page of its own and every link naming it landed on the index
        # instead, while `SECURITY.md`, which has no page at all, landed
        # there too. A link checker cannot see either, because both point at
        # something that exists.
        #
        # So the two cases are told apart. A document under `docs/` is meant
        # to have a page, and one missing from the map above is the map being
        # out of date — which stops the build rather than redirecting. Any
        # other `.md` is a repository file and is linked as one.
        if os.path.exists(os.path.join(ROOT, "docs", name + ".md")):
            raise SystemExit(
                "site: docs/%s.md has no entry in fix_href's map, so a link to "
                "it would silently become the docs index" % name
            )
        return "https://github.com/geneacta/keal/blob/main/" + href.lstrip("./") + anchor
    # Anything else still lives in the repository.
    return "https://github.com/geneacta/keal/blob/main/" + href.lstrip("./") + anchor


def inline(text):
    out = html.escape(text, quote=False)
    # Code spans first, so nothing inside them is re-read as markup.
    holes = []

    def stash(m):
        holes.append(m.group(1))
        return "\x00%d\x00" % (len(holes) - 1)

    out = INLINE_CODE.sub(stash, out)
    out = BOLD.sub(r"<strong>\1</strong>", out)
    out = ITALIC.sub(r"<em>\1</em>", out)
    out = LINK.sub(lambda m: '<a href="%s">%s</a>' % (fix_href(m.group(2)), m.group(1)), out)
    for i, code in enumerate(holes):
        out = out.replace("\x00%d\x00" % i, "<code>%s</code>" % code)
    return out


def markdown(text):
    """Markdown to HTML, plus the table of contents entries it passes."""
    lines = text.split("\n")
    out, toc = [], []
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("```"):
            lang = line[3:].strip()
            body = []
            i += 1
            while i < len(lines) and not lines[i].startswith("```"):
                body.append(lines[i])
                i += 1
            i += 1
            cls = " class=\"lang-%s\"" % lang if lang else ""
            out.append('<pre%s><code>%s</code></pre>' % (cls, html.escape("\n".join(body))))
            continue
        m = re.match(r"^(#{1,6})\s+(.*)$", line)
        if m:
            level, title = len(m.group(1)), m.group(2).strip()
            anchor = slug(re.sub(r"`", "", title))
            out.append("<h%d id=\"%s\">%s</h%d>" % (level, anchor, inline(title), level))
            if level == 2:
                toc.append((anchor, re.sub(r"`", "", title)))
            i += 1
            continue
        if re.match(r"^---+\s*$", line):
            out.append("<hr>")
            i += 1
            continue
        if line.startswith("|") and i + 1 < len(lines) and re.match(r"^\|[\s:|-]+\|?\s*$", lines[i + 1]):
            head = [c.strip() for c in line.strip().strip("|").split("|")]
            i += 2
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")])
                i += 1
            t = ["<div class=\"tablewrap\"><table><thead><tr>"]
            t += ["<th>%s</th>" % inline(c) for c in head]
            t.append("</tr></thead><tbody>")
            for r in rows:
                t.append("<tr>" + "".join("<td>%s</td>" % inline(c) for c in r) + "</tr>")
            t.append("</tbody></table></div>")
            out.append("".join(t))
            continue
        if re.match(r"^\s*[-*]\s+", line):
            items, indent_stack = [], None
            while i < len(lines) and (re.match(r"^\s*[-*]\s+", lines[i]) or (lines[i].startswith("  ") and lines[i].strip() and items)):
                if re.match(r"^\s*[-*]\s+", lines[i]):
                    items.append(re.sub(r"^\s*[-*]\s+", "", lines[i]))
                else:
                    items[-1] += " " + lines[i].strip()
                i += 1
            _ = indent_stack
            out.append("<ul>" + "".join("<li>%s</li>" % inline(x) for x in items) + "</ul>")
            continue
        if re.match(r"^\s*\d+\.\s+", line):
            items = []
            while i < len(lines) and (re.match(r"^\s*\d+\.\s+", lines[i]) or (lines[i].startswith("   ") and lines[i].strip() and items)):
                if re.match(r"^\s*\d+\.\s+", lines[i]):
                    items.append(re.sub(r"^\s*\d+\.\s+", "", lines[i]))
                else:
                    items[-1] += " " + lines[i].strip()
                i += 1
            out.append("<ol>" + "".join("<li>%s</li>" % inline(x) for x in items) + "</ol>")
            continue
        if line.startswith(">"):
            body = []
            while i < len(lines) and lines[i].startswith(">"):
                body.append(lines[i].lstrip(">").strip())
                i += 1
            out.append("<blockquote>%s</blockquote>" % inline(" ".join(body)))
            continue
        if not line.strip():
            i += 1
            continue
        para = []
        while i < len(lines) and lines[i].strip() and not lines[i].startswith(("#", "```", "|", ">")) \
                and not re.match(r"^\s*[-*]\s+", lines[i]) and not re.match(r"^\s*\d+\.\s+", lines[i]) \
                and not re.match(r"^---+\s*$", lines[i]):
            para.append(lines[i])
            i += 1
        if not para:
            # A line that opens no block and cannot start a paragraph — a
            # stray table row, a `#` without its space — would otherwise be
            # read forever. Take it as text and move on.
            para.append(lines[i])
            i += 1
        out.append("<p>%s</p>" % inline(" ".join(para)))
    return "\n".join(out), toc


# ---- page chrome ---------------------------------------------------------

# The last entry leaves the site: keal-view is a separate repository with a
# site of its own, and the French nav points at the French half of it. It is
# an absolute URL, which the template takes as it comes — nothing in `active`
# can match it, which is right, because you are never on it here.
NAV = {
    "en": [("index.html", "Home"), ("tour.html", "Tour"), ("docs.html", "Docs"),
           ("coming-from.html", "Coming from…"), ("stdlib.html", "Library"),
           ("https://geneacta.github.io/keal-view/", "keal-view")],
    "fr": [("index.html", "Accueil"), ("tour.html", "Le tour"), ("docs.html", "Docs"),
           ("coming-from.html", "Je viens de…"), ("stdlib.html", "Bibliothèque"),
           ("https://geneacta.github.io/keal-view/fr/", "keal-view")],
}

FOOTER = {
    "en": ("A statically typed, self-hosting programming language. Built by Geneacta.",
           "Source on GitHub", "Contribute", "Code of conduct", "Security"),
    "fr": ("Un langage de programmation typé statiquement et auto-hébergé. Construit par Geneacta.",
           "Les sources sur GitHub", "Contribuer", "Code de conduite", "Sécurité"),
}

SWITCH = {"en": ("fr/", "Français"), "fr": ("../", "English")}

# Where the site is served from. Canonical links, the language alternates
# and the sitemap all need an absolute address; a search engine reading a
# page cannot work out which of the two languages it is looking at, nor
# that the other one exists, from relative links alone.
BASE_URL = "https://geneacta.github.io/keal/"


def page(lang, filename, title, description, body, active=None, sidebar=None, toc=None):
    """One complete HTML page, in the site's dress."""
    prefix = "" if lang == "en" else "../"
    nav_links = []
    for href, label in NAV[lang]:
        cls = ' class="tab-active"' if href == active else ""
        nav_links.append('<a href="%s"%s>%s</a>' % (href, cls, label))
    other_href, other_label = SWITCH[lang]
    if lang == "en":
        other = "fr/" + filename
    else:
        other = "../" + filename
    foot = FOOTER[lang]

    aside = ""
    if sidebar:
        items = "".join(
            '<a href="%s"%s>%s</a>' % (h, ' class="on"' if h == filename else "", t)
            for h, t in sidebar
        )
        aside = '<div class="dside">%s</div>' % items
    tocbox = ""
    if toc:
        entries = "".join('<a href="#%s">%s</a>' % (a, html.escape(t)) for a, t in toc)
        head = "ON THIS PAGE" if lang == "en" else "SUR CETTE PAGE"
        tocbox = '<div class="dtoc"><div class="h">%s</div><div class="dtoc-items">%s</div></div>' % (head, entries)

    layout = body
    if sidebar or toc:
        layout = '<div class="dgrid">%s<div class="dmain prose">%s</div>%s</div>' % (aside, body, tocbox)

    return """<!doctype html>
<html lang="%(lang)s">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>%(title)s</title>
<meta name="description" content="%(desc)s">
<link rel="canonical" href="%(canonical)s">
<link rel="alternate" hreflang="en" href="%(alt_en)s">
<link rel="alternate" hreflang="fr" href="%(alt_fr)s">
<link rel="alternate" hreflang="x-default" href="%(alt_en)s">
<meta property="og:type" content="website">
<meta property="og:site_name" content="Keal">
<meta property="og:locale" content="%(locale)s">
<meta property="og:title" content="%(title)s">
<meta property="og:description" content="%(desc)s">
<meta property="og:url" content="%(canonical)s">
<meta property="og:image" content="%(image)s">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="%(title)s">
<meta name="twitter:description" content="%(desc)s">
<meta name="twitter:image" content="%(image)s">
<link rel="icon" type="image/png" href="%(prefix)sassets/keal3.png">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Sora:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600;700&display=swap" rel="stylesheet">
<link rel="stylesheet" href="%(prefix)sstyle.css">
</head>
<body>
<div class="wrap">
<nav class="nav">
  <div class="nav-left">
    <a href="%(home)s"><img class="nav-logo" src="%(prefix)sassets/keal.png" alt="Keal"></a>
    <div class="nav-links">%(links)s</div>
  </div>
  <div class="nav-right">
    <span class="badge">v1.2.0</span>
    <a class="btn-lang" href="%(other)s">%(other_label)s</a>
    <a class="btn-gh" href="https://github.com/geneacta/keal">GitHub</a>
  </div>
</nav>
%(body)s
<footer class="foot">
  <div class="foot-l">%(foot0)s</div>
  <div class="foot-r">
    <a href="https://github.com/geneacta/keal">%(foot1)s</a>
    <a href="https://github.com/geneacta/keal/blob/main/CONTRIBUTING.md">%(foot2)s</a>
    <a href="https://github.com/geneacta/keal/blob/main/CODE_OF_CONDUCT.md">%(foot3)s</a>
    <a href="https://github.com/geneacta/keal/blob/main/SECURITY.md">%(foot4)s</a>
  </div>
</footer>
</div>
<script src="%(prefix)ssite.js"></script>
</body>
</html>
""" % {
        "lang": lang,
        "title": html.escape(title),
        "desc": html.escape(description),
        "prefix": prefix,
        "canonical": BASE_URL + ("" if lang == "en" else "fr/") + filename,
        "alt_en": BASE_URL + filename,
        "alt_fr": BASE_URL + "fr/" + filename,
        "locale": "en_GB" if lang == "en" else "fr_FR",
        "image": BASE_URL + "assets/keal.png",
        "home": "index.html",
        "links": "".join(nav_links),
        "other": other,
        "other_label": other_label,
        "body": layout,
        "foot0": foot[0],
        "foot1": foot[1],
        "foot2": foot[2],
        "foot3": foot[3],
        "foot4": foot[4],
    }


def write(lang, filename, text):
    out_dir = SITE if lang == "en" else os.path.join(SITE, "fr")
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, filename), "w", encoding="utf-8", newline="") as f:
        f.write(text)
    return os.path.join(out_dir, filename)


# ---- the pages -----------------------------------------------------------

import content as C  # noqa: E402
import coming as CM  # noqa: E402
import bench as B  # noqa: E402


def code_window(title, code, output=None, run_label="Run"):
    out = ['<div class="cwin">', '<div class="cwin-bar"><span class="f">%s</span>' % html.escape(title)]
    if output is not None:
        out.append('<span class="run">▶ %s</span>' % run_label)
    out.append("</div>")
    out.append("<pre>%s</pre>" % html.escape(code))
    if output is not None:
        out.append('<div class="cwin-out"><pre>%s</pre></div>' % html.escape(output))
    out.append("</div>")
    return "".join(out)


def landing(lang):
    t = C.LANDING[lang]
    cards = "".join(
        '<div class="card"><h3>%s</h3><p>%s</p></div>' % (h, p) for h, p in t["cards"]
    )
    body = """
<section class="hero">
  <div class="pill">%(pill)s</div>
  <h1>%(h1)s</h1>
  <p class="lede">%(sub)s</p>
  <div class="cta">
    <a class="btn-primary" href="tour.html">%(cta1)s</a>
    <a class="btn-ghost" href="docs.html">%(cta2)s</a>
  </div>
  %(hero)s
</section>
<section class="cards">%(cards)s</section>
<section class="band">
  <h2>%(ih)s</h2>
  <p class="lede">%(ip)s</p>
  <div class="chips"><span>C</span><span>C++</span><span>Rust</span><span>Go</span><span>Java</span><span>Kotlin</span></div>
</section>
<section class="band">
  <h2>%(ph)s</h2>
  <p class="lede">%(pp)s</p>
  <div class="bars">
    <div class="barrow"><span class="bl">native</span><span class="bar"><div data-w="100%%"></div></span><span class="bv">1×</span></div>
    <div class="barrow"><span class="bl">bytecode VM</span><span class="bar"><div data-w="24%%"></div></span><span class="bv">84×</span></div>
    <div class="barrow"><span class="bl">tree-walker</span><span class="bar"><div data-w="9%%"></div></span><span class="bv">220×</span></div>
  </div>
  <p class="cap">%(pc)s</p>
</section>
<section class="band">
  <h2>%(sh)s</h2>
  %(start)s
  <p class="cap"><a href="tour.html">%(sa)s</a></p>
</section>
""" % {
        "pill": t["pill"], "h1": t["h1"], "sub": t["sub"], "cta1": t["cta1"], "cta2": t["cta2"],
        "hero": code_window("point.keal", C.HERO_CODE),
        "cards": cards, "ih": t["interop_h"], "ip": t["interop_p"],
        "ph": t["perf_h"], "pp": t["perf_p"], "pc": t["perf_cap"],
        "sh": t["start_h"],
        "start": code_window("shell", "git clone https://github.com/geneacta/keal\ncd keal\ncargo build --release\n./bootstrap.sh"),
        "sa": t["start_after"],
    }
    return page(lang, "index.html", t["title"], t["desc"], body, active="index.html")


def tour(lang):
    title = "Keal — the tour" if lang == "en" else "Keal — le tour"
    desc = ("Fifteen chapters, every snippet real and its output verified."
            if lang == "en" else
            "Quinze chapitres, chaque extrait réel et sa sortie vérifiée.")
    intro = ("Half an hour, top to bottom. Every snippet below is a real program and every"
             " output is what it actually prints — the suite checks them."
             if lang == "en" else
             "Une demi-heure, de haut en bas. Chaque extrait ci-dessous est un vrai programme"
             " et chaque sortie est ce qu'il imprime réellement — la suite les vérifie.")
    run = "Run" if lang == "en" else "Exécuter"
    chapters = []
    nav = []
    for i, ch in enumerate(C.TOUR, start=1):
        t_en, t_fr, b_en, b_fr, code, out = ch
        t = t_en if lang == "en" else t_fr
        b = b_en if lang == "en" else b_fr
        nav.append('<a class="tch" href="#c%d"><span class="n">%d</span>%s</a>' % (i, i, html.escape(t)))
        chapters.append(
            '<section class="chapter" id="c%d"><h2>%d. %s</h2><p class="lede">%s</p>%s</section>'
            % (i, i, html.escape(t), b, code_window("chapter%d.keal" % i, code, out, run))
        )
    body = """
<div class="tourgrid">
  <aside class="tournav">%(nav)s</aside>
  <div class="tourmain prose">
    <h1>%(title)s</h1>
    <p class="lede">%(intro)s</p>
    %(chapters)s
  </div>
</div>
""" % {"nav": "".join(nav), "title": "Tour of Keal" if lang == "en" else "Le tour de Keal",
       "intro": intro, "chapters": "".join(chapters)}
    return page(lang, "tour.html", title, desc, body, active="tour.html")


def sidebar_for(lang):
    out = []
    for href, label in C.SIDEBAR[lang]:
        if href == "GROUP":
            out.append(("GROUP", label))
        else:
            out.append((href, label))
    return out


def sidebar_html(lang, current):
    parts = []
    for href, label in C.SIDEBAR[lang]:
        if href == "GROUP":
            parts.append('<div class="grp">%s</div>' % label)
        else:
            cls = ' class="on"' if href == current else ""
            parts.append('<a href="%s"%s>%s</a>' % (href, cls, label))
    return '<div class="dside">%s</div>' % "".join(parts)


def doc_page(lang, source, filename, title):
    with open(os.path.join(ROOT, source), encoding="utf-8") as f:
        text = f.read()
    body, toc = markdown(text)
    note = ""
    if lang == "fr":
        note = ('<div class="callout"><span class="st">✦</span><p>Ce document de référence est'
                ' rédigé en anglais, la langue de travail du dépôt. Les pages du site — le tour,'
                ' les guides « je viens de… » et la bibliothèque — existent intégralement dans'
                ' les deux langues.</p></div>')
    head = "ON THIS PAGE" if lang == "en" else "SUR CETTE PAGE"
    entries = "".join('<a href="#%s">%s</a>' % (a, html.escape(t)) for a, t in toc)
    tocbox = '<div class="dtoc"><div class="h">%s</div><div class="dtoc-items">%s</div></div>' % (head, entries)
    layout = '<div class="dgrid">%s<div class="dmain prose">%s%s</div>%s</div>' % (
        sidebar_html(lang, filename), note, body, tocbox)
    return page(lang, filename, "Keal — " + title, title, layout, active="docs.html")


def docs_index(lang):
    title = "Documentation" if lang == "en" else "Documentation"
    lede = ("Everything the language promises, written down: the complete reference, the type"
            " rules, and the internals — memory, destruction, threads and the C boundary."
            if lang == "en" else
            "Tout ce que le langage promet, écrit noir sur blanc : la référence complète, les"
            " règles de typage, et les internes — mémoire, destruction, threads et frontière C.")
    cards = []
    for source, filename, t_en, t_fr, group in C.DOC_PAGES:
        t = t_en if lang == "en" else t_fr
        cards.append('<a class="card" href="%s"><h3>%s</h3><p>%s</p></a>' % (filename, html.escape(t), group))
    extra = ("stdlib.html", "The standard library" if lang == "en" else "La bibliothèque standard", "GUIDE")
    cards.append('<a class="card" href="%s"><h3>%s</h3><p>%s</p></a>' % extra)
    extra2 = ("coming-from.html", "Coming from another language" if lang == "en" else "Je viens d'un autre langage", "GUIDE")
    cards.append('<a class="card" href="%s"><h3>%s</h3><p>%s</p></a>' % extra2)
    body = '<section class="band"><h1>%s</h1><p class="lede">%s</p></section><section class="cards">%s</section>' % (
        title, lede, "".join(cards))
    return page(lang, "docs.html", "Keal — " + title, lede, body, active="docs.html")


def coming_index(lang):
    title = "Coming from another language" if lang == "en" else "Je viens d'un autre langage"
    lede = ("What you already write, and what it becomes here — plus the handful of things that"
            " will genuinely surprise you, which is the part a syntax table cannot carry."
            if lang == "en" else
            "Ce que vous écrivez déjà, et ce que cela devient ici — plus les quelques points qui"
            " vous surprendront vraiment, la part qu'un tableau de syntaxe ne porte pas.")
    cards = []
    for L in CM.LANGS:
        blurb = L["blurb_en"] if lang == "en" else L["blurb_fr"]
        cards.append('<a class="card" href="from-%s.html"><h3>%s</h3><p>%s</p></a>' % (L["key"], L["name"], blurb))
    body = '<section class="band"><h1>%s</h1><p class="lede">%s</p></section><section class="cards">%s</section>' % (
        title, lede, "".join(cards))
    return page(lang, "coming-from.html", "Keal — " + title, lede, body, active="coming-from.html")


def coming_page(lang, L):
    name = L["name"]
    title = ("Coming from %s" % name) if lang == "en" else ("Je viens de %s" % name)
    intro = L["intro_en"] if lang == "en" else L["intro_fr"]
    th = ("Concept", name, "Keal") if lang == "en" else ("Concept", name, "Keal")
    rows = []
    for c_en, c_fr, theirs, ours in L["rows"]:
        rows.append("<tr><td>%s</td><td><code>%s</code></td><td><code>%s</code></td></tr>"
                    % (html.escape(c_en if lang == "en" else c_fr),
                       html.escape(theirs), html.escape(ours)))
    table = ('<div class="tablewrap"><table><thead><tr><th>%s</th><th>%s</th><th>%s</th></tr></thead>'
             '<tbody>%s</tbody></table></div>' % (th[0], th[1], th[2], "".join(rows)))
    notes = []
    for t_en, t_fr, b_en, b_fr in L["notes"]:
        notes.append("<h2 id=\"%s\">%s</h2><p>%s</p>" % (
            slug(t_en), html.escape(t_en if lang == "en" else t_fr), b_en if lang == "en" else b_fr))
    heading = "What will surprise you" if lang == "en" else "Ce qui vous surprendra"
    body = "<h1>%s</h1><p class=\"lede\">%s</p>%s<h2 id=\"surprises\">%s</h2>%s" % (
        html.escape(title), intro, table, heading, "".join(notes))
    layout = '<div class="dgrid">%s<div class="dmain prose">%s</div></div>' % (
        sidebar_html(lang, "coming-from.html"), body)
    return page(lang, "from-%s.html" % L["key"], "Keal — " + title, intro[:150], layout,
                active="coming-from.html")


def benchmark(lang):
    """Four programs across eight languages, one section per machine.

    Absolute milliseconds belong to the machine that produced them, so the
    per-machine tables are never put side by side. The ratio to C is what
    travels, and it is the only thing a second machine is compared on — see
    the module docstring in `bench.py`.
    """
    T = B.TEXT[lang]
    E = html.escape
    L = lambda d, k: d[k + ("_en" if lang == "en" else "_fr")]
    out = []

    out.append('<h1>%s</h1><p class="lede">%s</p>' % (E(T["title"]), T["lede"]))

    rows = "".join(
        "<tr><td><code>%s</code></td><td>%s</td><td>%s</td></tr>"
        % (E(p["name"]), E(L(p, "size")), L(p, "what"))
        for p in B.PROGRAMS)
    out.append('<h2 id="programs">%s</h2><p>%s</p>'
               '<div class="tablewrap"><table><thead><tr><th>%s</th><th>%s</th><th>%s</th>'
               '</tr></thead><tbody>%s</tbody></table></div>'
               % (E(T["programs_h"]), T["programs_p"],
                  E(T["th_program"]), E(T["th_size"]), E(T["th_stresses"]), rows))

    for M in B.MACHINES:
        live = [n for n in B.LANGS if n in M["ms"]]
        out.append('<h2 id="m-%s">%s</h2>' % (M["key"], E("%s — %s" % (L(M, "name"), L(M, "cpu")))))
        out.append('<div class="chips"><span>%s</span><span>%s</span><span>%s</span>'
                   '<span>%d runs</span></div>'
                   % (E(M["os"]), E(M["date"]), E(M["keal"]), M["runs"]))
        # A machine may need to say something about its own run that the
        # toolchain table cannot carry — most usefully, that a version it
        # reports was arranged for the measurement and is not what the box
        # would give someone reproducing it with its own defaults.
        note = M.get("note_en" if lang == "en" else "note_fr")
        if note:
            out.append('<div class="callout"><span class="st">&#9670;</span><p>%s</p></div>'
                       % note)

        out.append("<h3>%s</h3><p>%s</p>" % (E(T["results_h"]), T["results_p"]))
        th = "".join("<th>%s</th>" % E(prog["name"]) for prog in B.PROGRAMS)
        body = []
        for n in live:
            cells = "".join("<td><code>%s</code></td>" % fmt_ms(v) for v in M["ms"][n])
            body.append("<tr><td>%s</td>%s<td><code>%s</code></td></tr>"
                        % (E(n), cells, fmt_ms(M["startup"].get(n, 0.0))))
        out.append('<div class="tablewrap"><table><thead><tr><th>%s</th>%s<th>%s</th></tr>'
                   '</thead><tbody>%s</tbody></table></div>'
                   % (E(T["th_lang"]), th, E(T["th_startup"]), "".join(body)))

        # A bar per language per program. The width is speed relative to the
        # fastest on that program, so the longest bar wins; the value beside it
        # is the ratio to C, which is what a second machine is compared on.
        for i, prog in enumerate(B.PROGRAMS):
            best = min(M["ms"][n][i] for n in live)
            base = M["ms"]["C"][i]
            bars = []
            for n in live:
                v = M["ms"][n][i]
                bars.append('<div class="barrow"><span class="bl">%s</span>'
                            '<span class="bar%s"><div data-w="%.1f%%"></div></span>'
                            '<span class="bv">%s</span></div>'
                            % (E(n), " hot" if n == B.SUBJECT else "",
                               max(best / v * 100.0, 0.6), fmt_ratio(v / base)))
            out.append('<p class="cap"><code>%s</code> — %s</p><div class="bars">%s</div>'
                       % (E(prog["name"]), E(L(prog, "size")), "".join(bars)))

        out.append("<h3>%s</h3><p>%s</p>" % (E(T["spread_h"]), T["spread_p"]))
        body = "".join("<tr><td>%s</td>%s</tr>"
                       % (E(n), "".join("<td><code>%d%%</code></td>" % v for v in M["spread"][n]))
                       for n in live)
        out.append('<div class="tablewrap"><table><thead><tr><th>%s</th>%s</tr></thead>'
                   '<tbody>%s</tbody></table></div>' % (E(T["th_lang"]), th, body))

        rows = "".join("<tr><td>%s</td><td>%s</td><td><code>%s</code></td></tr>"
                       % (E(n), E(v), E(f)) for n, v, f in M["toolchains"])
        out.append('<h3>%s</h3><div class="tablewrap"><table><thead><tr><th>%s</th>'
                   '<th>%s</th><th>%s</th></tr></thead><tbody>%s</tbody></table></div>'
                   % (E(T["toolchain_h"]), E(T["th_lang"]), E(T["th_version"]),
                      E(T["th_flags"]), rows))

    out.append('<h2 id="across">%s</h2>' % E(T["cross_h"]))
    if len(B.MACHINES) < 2:
        out.append("<p>%s</p>" % T["alone"])
    else:
        out.append("<p>%s</p>" % T["cross_p"])
        for i, prog in enumerate(B.PROGRAMS):
            th = "".join("<th>%s</th>" % E(L(M, "name")) for M in B.MACHINES)
            body = []
            for n in B.LANGS:
                if not any(n in M["ms"] for M in B.MACHINES):
                    continue
                cells = "".join(
                    ("<td><code>%s</code></td>" % fmt_ratio(M["ms"][n][i] / M["ms"]["C"][i]))
                    if n in M["ms"] else "<td>—</td>"
                    for M in B.MACHINES)
                body.append("<tr><td>%s</td>%s</tr>" % (E(n), cells))
            out.append('<p class="cap"><code>%s</code></p><div class="tablewrap"><table>'
                       '<thead><tr><th>%s</th>%s</tr></thead><tbody>%s</tbody></table></div>'
                       % (E(prog["name"]), E(T["th_lang"]), th, "".join(body)))

    # How many configurations crossed the order threshold, in the words the
    # sentence in `bench.py` leaves a hole for. Computed here so the claim and
    # the data cannot drift apart the way a hand-written one does.
    flagged = sum(M.get("order_effects", 0) for M in B.MACHINES)
    total = len(B.MACHINES) * len(B.LANGS) * len(B.PROGRAMS)
    n_en = ("Not one of %d crossed it" % total) if not flagged else ("%d of %d crossed it" % (flagged, total))
    n_fr = ("Pas un seul sur %d ne l'a franchi" % total) if not flagged else ("%d sur %d l'ont franchi" % (flagged, total))

    out.append('<h2 id="controls">%s</h2><p>%s</p>' % (E(T["controls_h"]), T["controls_p"]))
    cards = []
    for v_en, v_fr, t_en, t_fr, b_en, b_fr in B.CONTROLS:
        body = (b_en if lang == "en" else b_fr).replace("{n}", n_en if lang == "en" else n_fr)
        cards.append('<div class="card"><div class="eyebrow">%s</div><h3>%s</h3><p>%s</p></div>'
                     % (E(v_en if lang == "en" else v_fr),
                        E(t_en if lang == "en" else t_fr), body))
    out.append('<div class="cards3">%s</div>' % "".join(cards))

    out.append('<h2 id="limits">%s</h2>' % E(T["limits_h"]))
    for t_en, t_fr, b_en, b_fr in B.LIMITS:
        out.append("<p><strong>%s</strong> %s</p>"
                   % (t_en if lang == "en" else t_fr, b_en if lang == "en" else b_fr))

    out.append('<h2 id="method">%s</h2><p>%s</p>' % (E(T["method_h"]), T["method_p"]))

    toc = [("programs", T["programs_h"])]
    toc += [("m-" + M["key"], L(M, "name")) for M in B.MACHINES]
    toc += [("across", T["cross_h"]), ("controls", T["controls_h"]),
            ("limits", T["limits_h"]), ("method", T["method_h"])]
    head = "ON THIS PAGE" if lang == "en" else "SUR CETTE PAGE"
    entries = "".join('<a href="#%s">%s</a>' % (a, html.escape(t)) for a, t in toc)
    tocbox = ('<div class="dtoc"><div class="h">%s</div><div class="dtoc-items">%s</div></div>'
              % (head, entries))
    layout = '<div class="dgrid">%s<div class="dmain prose">%s</div>%s</div>' % (
        sidebar_html(lang, "benchmark.html"), "".join(out), tocbox)
    return page(lang, "benchmark.html", "Keal \u2014 " + T["title"], T["lede"][:150], layout,
                active="docs.html")


def fmt_ms(v):
    """Milliseconds, with a thin space once they stop being small."""
    return format(v, ",.0f").replace(",", "\u202f") if v >= 1000 else "%.1f" % v


def fmt_ratio(r):
    """A ratio to C. One decimal below ten, none above."""
    return ("%.0f\u00d7" % r) if r >= 10 else ("%.1f\u00d7" % r)


def stdlib(lang):
    """`keal doc`'s own output, re-dressed in the site's theme."""
    # `text=True` alone decodes the child in the machine's locale encoding,
    # which on a French Windows turned every em dash in the compiler's own
    # output into mojibake and wrote it into a page — without failing.
    raw = subprocess.run(
        [KEAL, "doc"], capture_output=True, text=True, encoding="utf-8", check=True
    ).stdout
    nav = re.search(r"<nav>(.*?)</nav>", raw, re.S).group(1)
    main = re.search(r"<main>(.*?)</main>", raw, re.S).group(1)
    main = re.sub(r"<h1>.*?</h1>\s*<p>.*?</p>", "", main, count=1, flags=re.S)
    main = re.sub(r"<footer>.*?</footer>", "", main, flags=re.S)
    title = "The standard library" if lang == "en" else "La bibliothèque standard"
    lede = ("Generated by <code>keal doc</code> from the prelude — the signatures below are the"
            " compiler's own."
            if lang == "en" else
            "Généré par <code>keal doc</code> depuis le prélude — les signatures ci-dessous sont"
            " celles du compilateur lui-même.")
    body = ('<div class="sgrid"><div class="snav">%s</div><div class="smain prose">'
            '<h1>%s</h1><p class="lede">%s</p>%s</div></div>' % (nav, title, lede, main))
    return page(lang, "stdlib.html", "Keal — " + title, "The Keal standard library", body,
                active="stdlib.html")


def main():
    written = []
    for lang in ("en", "fr"):
        written.append(write(lang, "index.html", landing(lang)))
        written.append(write(lang, "tour.html", tour(lang)))
        written.append(write(lang, "docs.html", docs_index(lang)))
        written.append(write(lang, "coming-from.html", coming_index(lang)))
        for L in CM.LANGS:
            written.append(write(lang, "from-%s.html" % L["key"], coming_page(lang, L)))
        for source, filename, t_en, t_fr, group in C.DOC_PAGES:
            written.append(write(lang, filename, doc_page(lang, source, filename,
                                                          t_en if lang == "en" else t_fr)))
        written.append(write(lang, "benchmark.html", benchmark(lang)))
        try:
            written.append(write(lang, "stdlib.html", stdlib(lang)))
        except Exception as e:  # a missing binary should not stop the rest
            print("  (stdlib skipped: %s)" % e)
    pages = sorted(
        (os.path.relpath(w, SITE).replace(os.sep, "/") for w in written),
        key=lambda p: (p.startswith("fr/"), p),
    )
    with open(os.path.join(SITE, "robots.txt"), "w", encoding="utf-8", newline="") as f:
        f.write("User-agent: *\nAllow: /\n\nSitemap: %ssitemap.xml\n" % BASE_URL)
    # One entry per page, each naming its counterpart in the other language,
    # so neither is read as a duplicate of the other.
    lines = ['<?xml version="1.0" encoding="UTF-8"?>',
             '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"'
             ' xmlns:xhtml="http://www.w3.org/1999/xhtml">']
    for rel in pages:
        name = rel[3:] if rel.startswith("fr/") else rel
        lines.append("  <url>")
        lines.append("    <loc>%s%s</loc>" % (BASE_URL, rel))
        lines.append('    <xhtml:link rel="alternate" hreflang="en" href="%s%s"/>' % (BASE_URL, name))
        lines.append('    <xhtml:link rel="alternate" hreflang="fr" href="%sfr/%s"/>' % (BASE_URL, name))
        lines.append("  </url>")
    lines.append("</urlset>")
    with open(os.path.join(SITE, "sitemap.xml"), "w", encoding="utf-8", newline="") as f:
        f.write("\n".join(lines) + "\n")
    check_links(written)
    print("%d pages written, plus robots.txt and sitemap.xml" % len(written))


def check_links(written):
    """Every relative link on every page written must reach something.

    A set of pages that promise each other exist is a promise nothing keeps:
    a document renamed, a heading retitled, a page dropped from one language
    and not the other, and the site still builds and still says the link is
    there. The build fails instead. Anchors are checked as anchors — the
    target page has to exist, and if the fragment names an `id`, that has to
    exist too, which is what catches a heading renamed in one language only.
    """
    import re as _re

    have = set()
    ids = {}
    for w in written:
        rel = os.path.relpath(w, SITE).replace(os.sep, "/")
        have.add(rel)
        with open(w, encoding="utf-8") as f:
            body = f.read()
        ids[rel] = set(_re.findall(r'id="([^"]+)"', body))
    # Anything already sitting in the output directory is reachable too —
    # the stylesheet, the images, the script. The first version of this check
    # counted only the pages it had just written and called 138 good links
    # broken, which is the failure mode of a checker that knows one half of
    # what it is checking.
    for base, _, names in os.walk(SITE):
        for n in names:
            rel = os.path.relpath(os.path.join(base, n), SITE).replace(os.sep, "/")
            have.add(rel)

    broken = []
    for w in written:
        rel = os.path.relpath(w, SITE).replace(os.sep, "/")
        here = os.path.dirname(rel)
        with open(w, encoding="utf-8") as f:
            body = f.read()
        for href in _re.findall(r'(?:href|src)="([^"]+)"', body):
            if href.startswith(("http://", "https://", "mailto:", "data:", "#")):
                continue
            target, _, anchor = href.partition("#")
            if not target:
                continue
            dest = os.path.normpath(os.path.join(here, target)).replace(os.sep, "/")
            if dest not in have:
                broken.append("%s -> %s" % (rel, href))
            elif anchor and dest in ids and anchor not in ids[dest]:
                broken.append("%s -> %s (no such anchor)" % (rel, href))
    if broken:
        for b in broken:
            print("  broken link: %s" % b)
        raise SystemExit("%d broken link(s); the site was not finished" % len(broken))


if __name__ == "__main__":
    sys.path.insert(0, SITE)
    main()
