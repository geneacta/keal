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
             "https://github.com/geneacta/kealsql"),
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
             "https://github.com/geneacta/kealsql"),
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
