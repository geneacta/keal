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
            "README": "index.html",
            "TUTORIAL": "tour.html",
            "CONTRIBUTING": "contributing.html",
        }
        return mapped.get(name, "docs.html") + anchor
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

NAV = {
    "en": [("index.html", "Home"), ("tour.html", "Tour"), ("docs.html", "Docs"),
           ("coming-from.html", "Coming from…"), ("stdlib.html", "Library")],
    "fr": [("index.html", "Accueil"), ("tour.html", "Le tour"), ("docs.html", "Docs"),
           ("coming-from.html", "Je viens de…"), ("stdlib.html", "Bibliothèque")],
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
    <span class="badge">v1.0.0</span>
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
    print("%d pages written, plus robots.txt and sitemap.xml" % len(written))


if __name__ == "__main__":
    sys.path.insert(0, SITE)
    main()
