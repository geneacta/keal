"""The site's own words, in both languages.

Everything here is authored (as opposed to converted from `docs/*.md`),
so it exists twice: once in English, once in French, with nothing said in
one language that is not said in the other.
"""

# ---- the landing page ----------------------------------------------------

LANDING = {
    "en": {
        "title": "Keal — the language that compiles itself",
        "desc": "A statically typed, self-hosting programming language: three engines agreeing byte for byte, native compilation through C11, and interop with six languages.",
        "pill": "Self-hosting — the bootstrap fixed point is verified on every run",
        "h1": "The language that compiles itself.",
        "sub": "Kotlin's shape over a C-family syntax, compiled to native code, with deterministic destruction and no garbage collector — and no borrow checker to argue with. Three engines that have to agree on every byte they print.",
        "cta1": "Start the tour →",
        "cta2": "Read the docs",
        "cards": [
            ("The compiler is written in Keal",
             "Lexer, parser, checker and C backend — each held byte-for-byte against its Rust oracle, and reproducing its own source."),
            ("Everything flows into Any, Any flows into nothing",
             "Inference everywhere, narrowing that survives <code>and</code>, early returns, even <code>implies</code>."),
            ("Six languages, one file",
             "C, C++, Rust, Go, Java and Kotlin all answer from a single Keal program."),
            ("Deterministic memory, no collector",
             "An object dies when its last reference does, at a statement boundary you can point at, and <code>deinit</code> runs there. No pause, no generation, no lifetime annotation."),
            ("One editor server, every editor",
             "<code>keal lsp</code> gives VS Code, Neovim, Helix and Zed the same thing: errors as you type, hover types, go to definition, rename. It reuses the compiler rather than modelling the language twice."),
        ],
        "interop_h": "One file, six languages.",
        "interop_p": "The four native ones meet Keal on the C ABI its binaries already speak — no runtime, no conversion layer. Java and Kotlin go through a gateway module, written in Keal.",
        "perf_h": "84× faster natively — with the same guarantees.",
        "perf_p": "The tree-walking interpreter is the specification. The bytecode VM is the default. <code>keal build</code> compiles through C11 to a real executable. The suite runs every program on all three and demands byte-identical output.",
        "perf_cap": "fib(35) — the same program, on all three engines.",
        "built_h": "Written in Keal",
        "built_p": "Three programs that use the language for what it is for, and report"
                   " what they find. Each has found defects the suite could not: a map that"
                   " looped after being emptied twice, a record that printed itself with a"
                   " pointer test for a value that is not one, a type the native backend"
                   " could not compile at all.",
        "built": [
            ("kealeb", "A web framework — routing, sessions, SQLite, live pages.",
             "https://geneacta.github.io/kealeb/"),
            ("keal-view", "A GUI framework — rasteriser, TrueType, layout, widgets.",
             "https://geneacta.github.io/keal-view/"),
            ("KealSql", "A language for PostgreSQL schemas and queries, compiled to SQL.",
             "https://geneacta.github.io/kealsql/"),
            # The only one whose page is here rather than on a site of its
            # own: Kealler's repository is private, so its downloads are cut
            # against this one and described on this site.
            ("Kealler", "The IDE — written in Keal, drawn by keal-view.",
             "kealler.html"),
        ],
        "start_h": "Running in a minute.",
        "start_after": "Then take the tour — 30 minutes, every snippet runs →",
    },
    "fr": {
        "title": "Keal — le langage qui se compile lui-même",
        "desc": "Un langage de programmation typé statiquement et auto-hébergé : trois moteurs d'accord à l'octet près, compilation native via C11, et interopérabilité avec six langages.",
        "pill": "Auto-hébergé — le point fixe du bootstrap est vérifié à chaque exécution",
        "h1": "Le langage qui se compile lui-même.",
        "sub": "La silhouette de Kotlin sur une syntaxe famille C, compilé en natif, avec une destruction déterministe et sans ramasse-miettes — et sans vérificateur d'emprunts à convaincre. Trois moteurs qui doivent s'accorder sur chaque octet imprimé.",
        "cta1": "Commencer le tour →",
        "cta2": "Lire la documentation",
        "cards": [
            ("Le compilateur est écrit en Keal",
             "Lexeur, parseur, vérificateur et backend C — tenus octet pour octet face à leurs oracles Rust, et reproduisant leur propre source."),
            ("Tout entre dans Any, Any ne sort nulle part",
             "Inférence partout, rétrécissement qui tient à travers <code>and</code>, les retours anticipés, et même <code>implies</code>."),
            ("Six langages, un seul fichier",
             "C, C++, Rust, Go, Java et Kotlin répondent tous depuis un unique programme Keal."),
            ("Mémoire déterministe, sans collecteur",
             "Un objet meurt quand meurt sa dernière référence, à une frontière d'instruction qu'on peut désigner, et <code>deinit</code> s'y exécute. Pas de pause, pas de génération, pas d'annotation de durée de vie."),
            ("Un serveur, tous les éditeurs",
             "<code>keal lsp</code> donne à VS Code, Neovim, Helix et Zed la même chose : les erreurs pendant qu'on tape, les types au survol, aller à la définition, renommer. Il réutilise le compilateur plutôt que de modéliser le langage deux fois."),
        ],
        "interop_h": "Un fichier, six langages.",
        "interop_p": "Les quatre natifs rencontrent Keal sur l'ABI C que ses binaires parlent déjà — pas de runtime, pas de couche de conversion. Java et Kotlin passent par un module passerelle, écrit en Keal.",
        "perf_h": "×84 en natif — avec les mêmes garanties.",
        "perf_p": "L'interpréteur arborescent est la spécification. La VM à bytecode est le défaut. <code>keal build</code> compile via C11 vers un vrai exécutable. La suite exécute chaque programme sur les trois et exige une sortie identique à l'octet.",
        "perf_cap": "fib(35) — le même programme, sur les trois moteurs.",
        "built_h": "Écrit en Keal",
        "built_p": "Trois programmes qui se servent du langage pour ce à quoi il sert, et"
                   " rapportent ce qu'ils trouvent. Chacun a trouvé des défauts que la suite"
                   " ne pouvait pas voir : une table qui bouclait après avoir été vidée deux"
                   " fois, un record qui s'imprimait avec un test de pointeur sur une valeur"
                   " qui n'en est pas une, un type que le backend natif ne savait pas"
                   " compiler du tout.",
        "built": [
            # Les deux ont une moitié française, comme la barre de navigation
            # le fait déjà pour keal-view : un lecteur français renvoyé vers
            # l'anglais est une couture qu'on ne voit pas en écrivant.
            ("kealeb", "Un cadriciel web — routage, sessions, SQLite, pages vivantes.",
             "https://geneacta.github.io/kealeb/fr/"),
            ("keal-view", "Un cadriciel graphique — rastériseur, TrueType, mise en page, widgets.",
             "https://geneacta.github.io/keal-view/fr/"),
            ("KealSql", "Un langage de schémas et de requêtes PostgreSQL, compilé vers SQL.",
             "https://geneacta.github.io/kealsql/fr/"),
            # Le seul dont la page est ici plutôt que sur un site à lui : le
            # dépôt de Kealler est privé, donc ses binaires sont produits par
            # celui-ci et décrits sur ce site.
            ("Kealler", "L'IDE — écrit en Keal, dessiné par keal-view.",
             "kealler.html"),
        ],
        "start_h": "Opérationnel en une minute.",
        "start_after": "Puis faites le tour — 30 minutes, chaque extrait s'exécute →",
    },
}

HERO_CODE = """class Point(val x: Float, val y: Float) {
    func length(): Float { sqrt(this.x * this.x + this.y * this.y) }
    func toString(): String { "(${this.x}, ${this.y})" }
}

func firstLong(points: List<Point>, min: Float): Point? {
    for (p in points) {
        if (p.length() > min) { return p }
    }
    return null
}

val found = firstLong([Point(1.0, 1.0), Point(3.0, 4.0)], 2.0)
println(when {
    found == null -> "nothing long enough"
    else -> "${found} has length ${found.length()}"
})"""

# ---- the tour ------------------------------------------------------------
# (title_en, title_fr, blurb_en, blurb_fr, code, output)

TOUR = [
    ("Hello, world", "Bonjour, monde",
     "A file is a program: top-level statements run in order, and there is no ceremony to get through first.",
     "Un fichier est un programme : les instructions de haut niveau s'exécutent dans l'ordre, sans cérémonie préalable.",
     'println("hello, world")\nval who = "Ada"\nprintln("hello ${who}, ${1 + 2} things")',
     "hello, world\nhello Ada, 3 things"),

    ("Values and bindings", "Valeurs et liaisons",
     "<code>val</code> binds once, <code>var</code> may be reassigned. Numbers copy; lists and maps are shared. There are no implicit numeric conversions.",
     "<code>val</code> lie une fois, <code>var</code> peut être réaffecté. Les nombres se copient ; listes et maps se partagent. Aucune conversion numérique implicite.",
     'val name = "Ada"\nvar count = 0\ncount += 1\n\nval n = 3\nval good = n.toFloat() / 2.0\nval ratio: Float = 1 / 2   // a literal adapts\n\nval xs = [1, 2]\nval ys = xs\nys.add(3)\nprintln("${good} ${ratio} ${xs}")',
     "1.5 0.5 [1, 2, 3]"),

    ("func and proc", "func et proc",
     "Which word you use says whether there is a result. A <code>func</code> must declare what it returns; a <code>proc</code> cannot — so <code>Unit</code> is never written by hand.",
     "Le mot employé dit s'il y a un résultat. Un <code>func</code> doit déclarer ce qu'il retourne ; un <code>proc</code> ne le peut pas — <code>Unit</code> ne s'écrit donc jamais à la main.",
     'func add(a: Int, b: Int): Int { a + b }\n\nproc greet(name: String, greeting: String = "hello") {\n    println("${greeting}, ${name}!")\n}\n\nprintln(add(2, 3))\ngreet("Ada")\ngreet("Ada", greeting = "hi")',
     "5\nhello, Ada!\nhi, Ada!"),

    ("Control flow", "Flot de contrôle",
     "Braces are mandatory and a block's value is its last expression, which is why <code>if</code> produces one. <code>unless (c)</code> is <code>if (not c)</code>.",
     "Les accolades sont obligatoires et la valeur d'un bloc est sa dernière expression — c'est pourquoi <code>if</code> en produit une. <code>unless (c)</code> vaut <code>if (not c)</code>.",
     'val n = -2\nval sign = if (n < 0) { "neg" } else { "pos" }\n\nfunc lengthOf(s: String?): Int {\n    unless (s != null) { return 0 }\n    return s.length\n}\n\nfor (i in 0..3) { println(i) }\nprintln("${sign} ${lengthOf(null)} ${lengthOf("abcd")}")',
     "0\n1\n2\nneg 0 4"),

    ("when", "when",
     "One construct covers what other languages split between <code>switch</code> and <code>match</code>: no fall-through, first arm wins, and it is an expression.",
     "Une seule construction couvre ce que d'autres langages séparent entre <code>switch</code> et <code>match</code> : pas de chute, le premier bras gagne, et c'est une expression.",
     'func describe(n: Int): String {\n    return when (n) {\n        0 -> "zero"\n        1, 2, 3 -> "small"\n        in 4..10 -> "medium"\n        else -> "large"\n    }\n}\nprintln(describe(2))\nprintln(describe(7))\nprintln(describe(99))',
     "small\nmedium\nlarge"),

    ("Null safety", "Sûreté face à null",
     "A type does not admit <code>null</code> unless you write <code>?</code>. After a check that proves something about an immutable binding, the fact holds — and Keal carries it further than most.",
     "Un type n'admet pas <code>null</code> sans <code>?</code>. Après un test qui prouve quelque chose sur une liaison immuable, le fait tient — et Keal le porte plus loin que la plupart.",
     'var maybe: String? = null\nprintln(maybe?.length)\nprintln(maybe ?: "default")\n\nval s: String? = "abc"\nif (s != null) { println(s.length) }\nprintln(s != null and s.length > 0)\nprintln(s != null implies s.length > 0)',
     "null\ndefault\n3\ntrue\ntrue"),

    ("Collections and lambdas", "Collections et lambdas",
     "Lists and maps are built in, with the higher-order methods you expect, typed generically.",
     "Listes et maps sont natives, avec les méthodes d'ordre supérieur attendues, typées génériquement.",
     'val xs = [1, 2, 3, 4]\nprintln(xs.map({ it * 2 }))\nprintln(xs.filter({ it % 2 == 0 }))\nprintln(xs.fold(0, { acc, x -> acc + x }))\n\nval ages = {"ada": 36, "alan": 41}\nfor (name in ages) { println("${name} is ${ages[name]!!}") }',
     "[2, 4, 6, 8]\n[2, 4]\n10\nada is 36\nalan is 41"),

    ("Records and classes", "Enregistrements et classes",
     "A <code>record</code> is the data case: immutable fields, structural equality, destructuring. A <code>class</code> is the one that can change.",
     "Un <code>record</code> est le cas données : champs immuables, égalité structurelle, déstructuration. Une <code>class</code> est celle qui peut changer.",
     'record Point(val x: Float, val y: Float)\nval a = Point(1.0, 2.0)\nval b = Point(1.0, 2.0)\nprintln(a == b)\nval Point(x, y) = a\nprintln("${x} ${y}")\n\nclass Counter(var n: Int) {\n    proc bump() { this.n += 1 }\n}\nval c = Counter(0)\nc.bump()\nprintln(c.n)',
     "true\n1.0 2.0\n1"),

    ("Generics and traits", "Génériques et traits",
     "Generics are monomorphised — no erasure, no boxing. A trait is a capability a type parameter can be required to have, not a type of its own.",
     "Les génériques sont monomorphisés — pas d'effacement, pas de boxing. Un trait est une capacité qu'on peut exiger d'un paramètre de type, pas un type en soi.",
     'func firstOr<T>(xs: List<T>, fallback: T): T {\n    for (x in xs) { return x }\n    return fallback\n}\nprintln(firstOr([1, 2], 0))\nprintln(firstOr(["a"], "z"))\n\nfunc total<T: Add>(xs: List<T>, zero: T): T {\n    var acc = zero\n    for (x in xs) { acc = acc + x }\n    return acc\n}\nprintln(total([1, 2, 3], 0))',
     "1\na\n6"),

    ("The eight connectives", "Les huit connecteurs",
     "Written as words, at one flat precedence, so a mixed expression must say what it means with parentheses.",
     "Écrits en toutes lettres, à une seule précédence, si bien qu'une expression mixte doit dire ce qu'elle veut dire avec des parenthèses.",
     'val a = true\nval b = false\nprintln(a and b)\nprintln(a or b)\nprintln(a xor b)\nprintln(a nand b)\nprintln(a nor b)\nprintln(a xnor b)\nprintln(a implies b)\nprintln(not a)',
     "false\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\nfalse"),

    ("Bits, in words", "Les bits, en toutes lettres",
     "An <code>Int</code> is 64 bits, and seven operators read it as those. Words, because <code>and</code>, <code>or</code> and <code>xor</code> already belong to <code>Bool</code>. They mix with nothing without parentheses — but they bind tighter than comparison, so the test everybody writes needs none.",
     "Un <code>Int</code>, c'est 64 bits, et sept opérateurs le lisent ainsi. En toutes lettres, parce que <code>and</code>, <code>or</code> et <code>xor</code> appartiennent déjà à <code>Bool</code>. Ils ne se mélangent à rien sans parenthèses — mais ils lient plus fort que la comparaison, si bien que le test que tout le monde écrit n'en demande aucune.",
     'val argb = (255 shl 24) bor (16 shl 16) bor (32 shl 8) bor 64\nprintln((argb ushr 16) band 0xFF)\nprintln(argb band 0xFF)\nprintln(0xF0 bxor 0xFF)\nprintln(bnot 0)\nval flag = 0x22\nprintln(flag band 2 != 0)',
     "16\n64\n15\n-1\ntrue"),

    ("deinit and weak", "deinit et weak",
     "<code>deinit</code> runs when the last reference dies, at the next statement boundary. <code>weak</code> writes the back edge of a cycle without holding it alive, so the cycle still dies.",
     "<code>deinit</code> s'exécute quand la dernière référence meurt, à la frontière d'instruction suivante. <code>weak</code> écrit l'arête arrière d'un cycle sans la maintenir en vie — le cycle meurt quand même.",
     'var freed = 0\nclass Item(val id: Int) {\n    weak var owner: Owner? = null\n    proc deinit() { freed += 1 }\n}\nclass Owner(val id: Int) {\n    var held: Item? = null\n    proc deinit() { freed += 1 }\n}\nproc pair() {\n    val o = Owner(1)\n    val it = Item(2)\n    o.held = it\n    it.owner = o\n}\npair()\nprintln("freed ${freed}")',
     "freed 2"),

    ("Native code and C", "Code natif et C",
     "<code>keal build</code> compiles through C11 to a real executable, and what it cannot compile it refuses by name — it never mis-compiles.",
     "<code>keal build</code> compile via C11 vers un vrai exécutable, et ce qu'il ne peut pas compiler, il le refuse en le nommant — il ne compile jamais de travers.",
     'native """\n#include <math.h>\nstatic double keal_hypot(double a, double b) { return hypot(a, b); }\n"""\n\nextern func hypot(a: Float, b: Float): Float = "keal_hypot"\n\nprintln(hypot(3.0, 4.0))',
     "5.0"),

    ("constexpr", "constexpr",
     "A promise about <em>when</em> the work happens: the compiler runs it and writes the answer into the program as a literal. Where it cannot, it refuses by name rather than quietly leaving the work for run time — and it always finishes, because a compiler that never answers is not a tool.",
     "Une promesse sur le <em>moment</em> où le travail a lieu : le compilateur l'exécute et écrit la réponse dans le programme, sous forme de littéral. Là où il ne peut pas, il refuse en le nommant plutôt que de laisser discrètement le travail à l'exécution — et il termine toujours, car un compilateur qui ne répond jamais n'est pas un outil.",
     'constexpr func squares(n: Int): List<Int> {\n    var out: List<Int> = []\n    for (i in 1..n) { out.add(i * i) }\n    return out\n}\n\nconstexpr val KB = 1024\nconstexpr val TABLE: List<Int> = squares(8)\nprintln("${KB * KB} ${TABLE.size} ${TABLE[6]}")',
     "1048576 7 49"),

    ("enum", "enum",
     "A closed set of names. The checker knows every value the type has, so a <code>when</code> over one needs no <code>else</code> — and the day somebody adds a variant, every <code>when</code> that forgot it is an error rather than a surprise at run time.",
     "Un ensemble fermé de noms. Le vérificateur connaît toutes les valeurs du type, donc un <code>when</code> sur l'un d'eux n'a pas besoin de <code>else</code> — et le jour où quelqu'un ajoute une variante, chaque <code>when</code> qui l'a oubliée est une erreur plutôt qu'une surprise à l'exécution.",
     'enum Suit { Hearts, Diamonds, Clubs, Spades }\n\nfunc isRed(s: Suit): Bool {\n    return when (s) {\n        Suit.Hearts, Suit.Diamonds -> true\n        Suit.Clubs, Suit.Spades -> false\n    }\n}\nprintln("${Suit.Hearts} ${isRed(Suit.Hearts)} ${isRed(Suit.Spades)}")',
     "Hearts true false"),

    ("Macros", "Macros",
     "A named piece of syntax, spliced where it is written. The <code>!</code> is not decoration: a macro may assign to what it was given, run an argument twice or never, and let a <code>return</code> pass through to the function around it — three things a call cannot do.",
     "Un morceau de syntaxe nommé, inséré là où il est écrit. Le <code>!</code> n'est pas décoratif : une macro peut affecter ce qu'on lui donne, exécuter un argument deux fois ou jamais, et laisser un <code>return</code> traverser jusqu'à la fonction autour — trois choses qu'un appel ne peut pas faire.",
     'macro swap(a, b) {\n    val held = a\n    a = b\n    b = held\n}\n\nmacro guard(cond, fallback) {\n    unless (cond) { return fallback }\n}\n\nfunc describe(n: Int): String {\n    guard!(n > 0, "not positive")\n    return "ok"\n}\n\nvar p = 1\nvar q = 2\nswap!(p, q)\nprintln("${p} ${q} ${describe(-3)} ${describe(7)}")',
     "2 1 not positive ok"),
]

# ---- the reference documents converted from docs/ ------------------------
# (source, filename, title_en, title_fr, group)

DOC_PAGES = [
    ("docs/language.md", "language.html", "The complete reference", "La référence complète", "LANGUAGE"),
    ("docs/types.md", "types.html", "Types and inference", "Types et inférence", "LANGUAGE"),
    ("docs/memory.md", "memory.html", "The memory model", "Le modèle mémoire", "INTERNALS"),
    ("docs/threads.md", "threads.html", "Threads and actors", "Threads et acteurs", "INTERNALS"),
    ("docs/drop.md", "deinit.html", "Deterministic deinit", "deinit déterministe", "INTERNALS"),
    ("docs/interop.md", "interop.html", "Interop: C to Kotlin", "Interop : de C à Kotlin", "INTERNALS"),
    ("docs/packages.md", "packages.html", "Packages and namespaces", "Paquets et espaces de noms", "LANGUAGE"),
    ("CONTRIBUTING.md", "contributing.html", "Contributing", "Contribuer", "GUIDE"),
]

SIDEBAR = {
    "en": [
        ("GROUP", "GUIDE"),
        ("tour.html", "Tour of Keal"),
        ("stdlib.html", "Standard library"),
        ("coming-from.html", "Coming from another language"),
        ("contributing.html", "Contributing"),
        ("benchmark.html", "Benchmark"),
        ("GROUP", "LANGUAGE"),
        ("language.html", "The complete reference"),
        ("types.html", "Types and inference"),
        ("packages.html", "Packages and namespaces"),
        ("GROUP", "INTERNALS"),
        ("memory.html", "The memory model"),
        ("deinit.html", "Deterministic deinit"),
        ("threads.html", "Threads and actors"),
        ("interop.html", "Interop: C to Kotlin"),
    ],
    "fr": [
        ("GROUP", "GUIDE"),
        ("tour.html", "Le tour de Keal"),
        ("stdlib.html", "Bibliothèque standard"),
        ("coming-from.html", "Je viens d'un autre langage"),
        ("contributing.html", "Contribuer"),
        ("benchmark.html", "Banc d'essai"),
        ("GROUP", "LANGAGE"),
        ("language.html", "La référence complète"),
        ("types.html", "Types et inférence"),
        ("packages.html", "Paquets et espaces de noms"),
        ("GROUP", "INTERNES"),
        ("memory.html", "Le modèle mémoire"),
        ("deinit.html", "deinit déterministe"),
        ("threads.html", "Threads et acteurs"),
        ("interop.html", "Interop : de C à Kotlin"),
    ],
}

# ------------------------------------------------------------ kealler ---
# The IDE lives in a private repository; its binaries do not, because a
# private repository's release assets need a token to download, which is no
# way to hand somebody a program. They are cut as releases here, tagged
# `kealler-vN` so they cannot be mistaken for a release of the language, and
# this page points at them.
#
# Empty until the first one is cut. The page then says what Kealler is and
# what it is waiting for, rather than offering a download that does not exist
# — and setting this to a version is the whole of publishing it.
KEALLER_VERSION = "0.2.1"

KEALLER_BUILDS = [
    ("kealler-macos-arm64", "macOS", "Apple silicon"),
    ("kealler-linux-x86_64", "Linux", "x86-64"),
    # Linux on ARM belongs here and is not here yet. 0.1.1's arm64 job failed
    # and was allowed to fail so the other three could be published, so the
    # file does not exist and this row would offer a 404 — worse than a page
    # that says to wait, because the reader spends their trust before finding
    # out. `site/checkdownloads.py` asks GitHub about every row and refuses
    # exactly this. Put it back the moment the archive is attached.
    # ("kealler-linux-arm64", "Linux", "ARM64"),
    ("kealler-windows-x86_64", "Windows", "x86-64"),
]

KEALLER = {
    "en": {
        "title": "Kealler — the Keal IDE",
        "lede": "An editor for Keal, written in Keal and drawn by keal-view. There is no toolkit "
                "under it: every pixel, the gutter and the syntax colours included, comes out of "
                "a loop written in Keal.",
        "h_get": "Download",
        "waiting": "**Not released yet.** Kealler reads, colours and checks a Keal project today "
                   "— what it cannot do is let you type into it, because keal-view is still "
                   "growing text selection across more than one line. A download called an IDE "
                   "that opens a file and refuses a keystroke would be a promise this page had no "
                   "business making, so the first release waits for that.",
        "windows": "<b>On Windows, fetch it with <code>curl</code> rather than through this "
                   "page.</b> Windows marks what a browser downloads and SmartScreen reads that "
                   "mark, not the file, so an unsigned program gets <i>“Windows protected your "
                   "PC”</i> with the way through hidden behind “More info”. A terminal fetch "
                   "attaches no mark. If you already downloaded it here, "
                   "<code>Unblock-File .\\kealler.exe</code> in PowerShell removes it.",
        "macos": "<b>On macOS, fetch it with <code>curl</code> rather than through this page.</b> "
                 "macOS quarantines what a browser downloads and Gatekeeper reads that attribute, "
                 "not the file — and this build is signed only ad-hoc, so it will object. "
                 "<code>curl</code> sets no such attribute, so there is nothing to override: "
                 "<code>curl -L -o kealler.tar.gz &lt;the link below&gt; &amp;&amp; tar xzf "
                 "kealler.tar.gz &amp;&amp; ./kealler</code>. If you already downloaded it here, "
                 "<code>xattr -dr com.apple.quarantine kealler</code> removes it.",
        "h_what": "What it does",
        "does": [
            ("Open, edit, save",
             "A file tree with a filter that matches letters in order — <code>apk</code> finds "
             "<code>app.keal</code>. Undo grouped the way an editor groups, so undoing a word "
             "takes one press and not five. Find across every file in the tree, with the matches "
             "painted over the syntax rather than instead of it. A file's line endings are kept: "
             "a Windows file opened here stays a Windows file."),
            ("Coloured by the compiler, not by a copy of it",
             "The highlighting is <code>keal tokens</code> — the lexer that compiles the file. A "
             "second grammar written in regular expressions agrees with the compiler on the day "
             "it is written and disagrees the first time the language grows."),
            ("What the compiler thinks, where it happened",
             "<code>keal check</code> runs on the file you are looking at, and each thing it "
             "found is drawn on the line it names rather than in a list somewhere else."),
            ("A preview that is the program's own output",
             "Building a keal-view program and showing the frame it drew — with no display "
             "involved, through <code>--snapshot</code>. No second renderer to keep in agreement "
             "with the first, so the preview cannot be subtly wrong."),
            ("A layout that survives being closed",
             "Panels are dragged, split and tabbed, and where you left them is where they are. "
             "An arrangement naming a panel that has gone is pruned rather than refused."),
        ],
        "h_needs": "What it needs",
        "needs": "The Keal compiler, beside it or on your path — the colouring is <code>keal "
                 "tokens</code>, the diagnostics are <code>keal check</code>, and building is "
                 "<code>keal build</code>. With none it says so and keeps working: the tree, the "
                 "source and the layout do not need one, the colouring goes plain, and nothing is "
                 "checked. That is the honest state rather than a silent one.",
        "h_platform": "Platform",
        "h_arch": "Architecture",
        "h_file": "File",
    },
    "fr": {
        "title": "Kealler — l'IDE de Keal",
        "lede": "Un éditeur pour Keal, écrit en Keal et dessiné par keal-view. Il n'y a aucune "
                "boîte à outils dessous : chaque pixel, la gouttière et la coloration comprises, "
                "sort d'une boucle écrite en Keal.",
        "h_get": "Téléchargement",
        "waiting": "**Pas encore publié.** Kealler lit, colore et vérifie un projet Keal "
                   "aujourd'hui — ce qu'il ne sait pas faire, c'est vous laisser y taper, parce "
                   "que keal-view apprend encore la sélection de texte sur plusieurs lignes. Un "
                   "téléchargement appelé « IDE » qui ouvre un fichier et refuse une frappe "
                   "serait une promesse que cette page n'a pas à faire, donc la première version "
                   "attend cela.",
        "windows": "<b>Sur Windows, récupérez-le avec <code>curl</code> plutôt que par cette "
                   "page.</b> Windows marque ce qu'un navigateur télécharge et SmartScreen lit "
                   "cette marque, pas le fichier — donc un programme non signé reçoit "
                   "<i>« Windows a protégé votre ordinateur »</i>, la sortie étant cachée "
                   "derrière « Informations complémentaires ». Une récupération en terminal ne "
                   "pose aucune marque. Si vous l'avez déjà téléchargé ici, "
                   "<code>Unblock-File .\\kealler.exe</code> dans PowerShell la retire.",
        "macos": "<b>Sur macOS, récupérez-le avec <code>curl</code> plutôt que par cette "
                 "page.</b> macOS met en quarantaine ce qu'un navigateur télécharge, et "
                 "Gatekeeper lit cet attribut, pas le fichier — et ce binaire n'est signé qu'en "
                 "ad-hoc, donc il protestera. <code>curl</code> ne pose aucun attribut, donc il "
                 "n'y a rien à outrepasser : <code>curl -L -o kealler.tar.gz &lt;le lien "
                 "ci-dessous&gt; &amp;&amp; tar xzf kealler.tar.gz &amp;&amp; ./kealler</code>. "
                 "Si vous l'avez déjà téléchargé ici, <code>xattr -dr com.apple.quarantine "
                 "kealler</code> le retire.",
        "h_what": "Ce qu'il fait",
        "does": [
            ("Ouvrir, éditer, enregistrer",
             "Un arbre de fichiers avec un filtre qui prend les lettres dans l'ordre — "
             "<code>apk</code> trouve <code>app.keal</code>. Une annulation groupée comme un "
             "éditeur groupe : annuler un mot demande une pression et non cinq. Une recherche "
             "dans tous les fichiers de l'arbre, les correspondances peintes par-dessus la "
             "syntaxe et non à sa place. Les fins de ligne d'un fichier sont conservées : un "
             "fichier Windows ouvert ici reste un fichier Windows."),
            ("Coloré par le compilateur, pas par une copie de lui",
             "La coloration est <code>keal tokens</code> — le lexer qui compile le fichier. Une "
             "seconde grammaire écrite en expressions régulières est d'accord avec le compilateur "
             "le jour où on l'écrit et en désaccord dès que le langage grandit."),
            ("Ce que le compilateur pense, là où ça se passe",
             "<code>keal check</code> tourne sur le fichier ouvert, et chaque chose trouvée est "
             "dessinée sur la ligne qu'elle nomme plutôt que dans une liste ailleurs."),
            ("Un aperçu qui est la sortie du programme lui-même",
             "Compiler un programme keal-view et montrer l'image qu'il a dessinée — sans aucun "
             "écran, par <code>--snapshot</code>. Pas de second moteur de rendu à tenir en accord "
             "avec le premier, donc l'aperçu ne peut pas être subtilement faux."),
            ("Une disposition qui survit à la fermeture",
             "Les panneaux se traînent, se divisent et s'empilent en onglets, et là où vous les "
             "avez laissés est là où ils sont. Un agencement qui nomme un panneau disparu est "
             "élagué plutôt que refusé."),
        ],
        "h_needs": "Ce qu'il lui faut",
        "needs": "Le compilateur Keal, à côté de lui ou dans votre chemin — la coloration est "
                 "<code>keal tokens</code>, les diagnostics sont <code>keal check</code>, et la "
                 "compilation est <code>keal build</code>. Sans aucun il le dit et continue : "
                 "l'arborescence, la source et la disposition n'en ont pas besoin, la coloration "
                 "s'éteint, et rien n'est vérifié. C'est l'état honnête plutôt que l'état "
                 "silencieux.",
        "h_platform": "Plateforme",
        "h_arch": "Architecture",
        "h_file": "Fichier",
    },
}


# ---- the step-by-step page -----------------------------------------------
# A first Keal program, from nothing installed to a native binary, for a
# reader who has never seen the language and may never have used a compiler.
#
# (title_en, title_fr, body_en, body_fr, [(label, code, output_or_None), ...])
#
# Every transcript below was run on a real machine before it was written
# down; `{version}` is filled in from Cargo.toml at build time so the page
# cannot drift past the compiler it describes. What cannot be verified from
# here — the download, the PATH, another platform's C compiler — is written
# as an instruction and never as an output.

STEPS_LEDE = {
    "en": "Nothing installed to a program of your own, in eight steps. Every command is"
          " written out in full, and every output is what the command actually prints.",
    "fr": "De rien d'installé à votre propre programme, en huit étapes. Chaque commande est"
          " écrite en entier, et chaque sortie est ce que la commande imprime réellement.",
}

STEPS = [
    ("Get the compiler", "Récupérer le compilateur",
     "Keal is one binary. It carries the prelude and the C runtime inside it, so there is nothing"
     " to install beside it. Take the archive for your machine from the"
     " <a href=\"https://github.com/geneacta/keal/releases/latest\">latest release</a>, unpack it,"
     " and put the <code>keal</code> file somewhere on your <code>PATH</code>."
     "<br><br><strong>Linux</strong> needs glibc 2.34 or newer — Ubuntu 22.04, Debian 12, RHEL 9"
     " and later. <strong>macOS</strong> downloads are unsigned, so clear the quarantine flag once."
     " There is no prebuilt archive for <strong>Linux on ARM</strong>; on that machine, and on any"
     " platform not in the list, build from source instead — it is the step below and it works"
     " everywhere Rust does.",
     "Keal est un seul binaire. Il porte le prélude et le runtime C en lui, il n'y a donc rien à"
     " installer à côté. Prenez l'archive de votre machine dans la"
     " <a href=\"https://github.com/geneacta/keal/releases/latest\">dernière version publiée</a>,"
     " décompressez-la, et placez le fichier <code>keal</code> quelque part dans votre"
     " <code>PATH</code>."
     "<br><br><strong>Linux</strong> demande la glibc 2.34 ou plus récente — Ubuntu 22.04,"
     " Debian 12, RHEL 9 et suivantes. Les téléchargements <strong>macOS</strong> ne sont pas"
     " signés : il faut retirer l'attribut de quarantaine une fois. Il n'existe pas d'archive"
     " pour <strong>Linux sur ARM</strong> ; sur cette machine, et sur toute plateforme absente"
     " de la liste, compilez depuis les sources — c'est l'étape ci-dessous et elle marche partout"
     " où Rust marche.",
     [("macOS — once, after unpacking", "xattr -d com.apple.quarantine keal", None),
      ("Building from source, anywhere Rust runs",
       "git clone https://github.com/geneacta/keal.git\n"
       "cd keal\n"
       "cargo build --release\n"
       "cargo install --path .", None)]),

    ("Check it is really there", "Vérifier qu'il est bien là",
     "Two commands, and they answer different questions. <code>keal version</code> says the binary"
     " is on your <code>PATH</code> and runs. <code>keal doctor</code> says what it can do on this"
     " machine: a C compiler unlocks <code>keal build</code>, and the rest are for talking to other"
     " languages. Nothing here is required to run a program — the first four steps need none of it.",
     "Deux commandes, et elles répondent à des questions différentes. <code>keal version</code> dit"
     " que le binaire est dans votre <code>PATH</code> et qu'il s'exécute. <code>keal doctor</code>"
     " dit ce qu'il sait faire sur cette machine : un compilateur C débloque <code>keal build</code>,"
     " le reste sert à parler aux autres langages. Rien de tout cela n'est requis pour exécuter un"
     " programme — les quatre premières étapes n'en ont besoin d'aucun.",
     [("keal version", "keal version", "keal {version}"),
      ("keal doctor", "keal doctor",
       "keal doctor — the interop toolchains on this machine\n"
       "\n"
       "  cc       cc (Ubuntu 15.2.0-16ubuntu1) 15.2.0\n"
       "             verified against: Apple clang 21.0.0\n"
       "             unlocks: keal build (required for native)\n"
       "\n"
       "  cargo    cargo 1.98.0 (797e8a9bc 2026-08-05)\n"
       "             verified against: rustc 1.98.0\n"
       "             unlocks: building the toolchain (required)\n"
       "\n"
       "  go       MISSING            — Go interop (c-archive)")]),

    ("Write the file", "Écrire le fichier",
     "A file is a program. There is no class to declare, no <code>main</code> to write, no project"
     " to create first: statements at the top level run in the order they are written. Put this in"
     " a file called <code>hello.keal</code>, anywhere you like.",
     "Un fichier est un programme. Aucune classe à déclarer, aucun <code>main</code> à écrire,"
     " aucun projet à créer d'abord : les instructions de premier niveau s'exécutent dans l'ordre"
     " où elles sont écrites. Mettez ceci dans un fichier nommé <code>hello.keal</code>, où vous"
     " voulez.",
     [("hello.keal", 'println("hello, world")', None)]),

    ("Run it", "L'exécuter",
     "That is the whole cycle: write, run. No build step, no configuration file, no directory"
     " layout the tool insists on.",
     "Voilà tout le cycle : écrire, exécuter. Pas d'étape de compilation, pas de fichier de"
     " configuration, pas d'arborescence imposée par l'outil.",
     [("keal hello.keal", "keal hello.keal", "hello, world")]),

    ("When it is wrong", "Quand ça ne va pas",
     "Sooner rather than later, and that is the point. Keal is statically typed: the mistake below"
     " is caught before anything runs, and <code>keal check</code> asks for that check without"
     " running the program at all. The error names the file, the line, the column, what it found"
     " and what it expected.",
     "Tôt plutôt que tard, et c'est bien l'intention. Keal est typé statiquement : l'erreur"
     " ci-dessous est attrapée avant que quoi que ce soit ne s'exécute, et <code>keal check</code>"
     " demande cette vérification sans exécuter le programme du tout. L'erreur nomme le fichier,"
     " la ligne, la colonne, ce qu'elle a trouvé et ce qu'elle attendait.",
     [("oops.keal", 'val n: Int = "quarante-deux"\nprintln(n)', None),
      ("keal check oops.keal", "keal check oops.keal",
       "error: initializer has type `String`, but `Int` was expected\n"
       "  --> oops.keal:1:14\n"
       "  |\n"
       "1 | val n: Int = \"quarante-deux\"\n"
       "  |              ^\n"
       "1 error found")]),

    ("The same file, three ways", "Le même fichier, trois façons",
     "Keal has three engines, and all three run the source you just wrote. <code>keal</code> alone"
     " uses the bytecode VM — the default, and the fast way to run something now."
     " <code>--ast</code> uses the tree-walking interpreter, which is the specification the other"
     " two are checked against. <code>keal build</code> compiles through C to a real executable"
     " that needs no Keal installed to run — that one, and only that one, needs a C compiler."
     "<br><br>They must print the same bytes. That is not a slogan: it is what the test suite"
     " checks on every program it has, and it is how nearly every defect in this compiler has been"
     " found — one engine disagreeing with the other two.",
     "Keal a trois moteurs, et les trois exécutent la source que vous venez d'écrire."
     " <code>keal</code> seul utilise la machine virtuelle à bytecode — le défaut, et le moyen"
     " rapide d'exécuter quelque chose tout de suite. <code>--ast</code> utilise l'interprète à"
     " parcours d'arbre, qui est la spécification contre laquelle les deux autres sont vérifiés."
     " <code>keal build</code> compile en passant par C vers un vrai exécutable qui n'a besoin"
     " d'aucun Keal installé pour tourner — celui-là, et lui seul, demande un compilateur C."
     "<br><br>Les trois doivent imprimer les mêmes octets. Ce n'est pas un slogan : c'est ce que"
     " la suite de tests vérifie sur chaque programme qu'elle possède, et c'est ainsi que presque"
     " tous les défauts de ce compilateur ont été trouvés — un moteur en désaccord avec les deux"
     " autres.",
     [("The three engines, one file",
       "keal hello.keal          # the bytecode VM, the default\n"
       "keal --ast hello.keal    # the tree-walking interpreter\n"
       "keal build hello.keal    # a native executable, through C\n"
       "./hello",
       "hello, world\nhello, world\nhello\nhello, world")]),

    ("A file you can just execute", "Un fichier directement exécutable",
     "A Keal file can name its own interpreter on the first line, which makes it an ordinary"
     " executable on macOS and Linux. Nothing else changes: it is the same language and the same"
     " file, and <code>keal build</code> still turns it into a binary the day you want one.",
     "Un fichier Keal peut nommer son propre interprète sur la première ligne, ce qui en fait un"
     " exécutable ordinaire sur macOS et Linux. Rien d'autre ne change : c'est le même langage et"
     " le même fichier, et <code>keal build</code> en fera toujours un binaire le jour où vous en"
     " voudrez un.",
     [("script.keal", '#!/usr/bin/env keal\nprintln("hello from a script")', None),
      ("Make it executable, then run it",
       "chmod +x script.keal\n./script.keal", "hello from a script")]),

    ("Where to go from here", "Où aller ensuite",
     "The <a href=\"tour.html\">tour</a> is fifteen chapters and about half an hour, every snippet"
     " a real program with its real output. <a href=\"docs.html\">The docs</a> are the reference"
     " once you want the rules rather than the taste of it. <code>keal repl</code> gives you a"
     " prompt to try one line at a time. And if you already know another language, the"
     " <a href=\"coming-from.html\">coming from…</a> guides start from what you already do.",
     "Le <a href=\"tour.html\">tour</a> fait quinze chapitres et environ une demi-heure, chaque"
     " extrait étant un vrai programme avec sa vraie sortie. <a href=\"docs.html\">Les docs</a>"
     " sont la référence quand vous voudrez les règles plutôt que le goût du langage."
     " <code>keal repl</code> ouvre une invite pour essayer une ligne à la fois. Et si vous"
     " connaissez déjà un autre langage, les guides"
     " <a href=\"coming-from.html\">je viens de…</a> partent de ce que vous savez déjà.",
     [("keal repl", "keal repl", None)]),
]


# The line under Kealler's page. The site's own says "a statically typed,
# self-hosting programming language", which is true of Keal and not of an
# editor — and on a page wearing Kealler's bar it was the one place still
# saying whose site this really is. It names Kealler and then says where
# Kealler belongs, which is the honest version of both.
KEALLER_FOOT = {
    "en": "Kealler — an editor for Keal, written in Keal. Part of the Keal project, by Geneacta.",
    "fr": "Kealler — un éditeur pour Keal, écrit en Keal. Fait partie du projet Keal, par Geneacta.",
}
