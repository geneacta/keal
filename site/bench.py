"""The cross-language benchmark: four programs, eight languages, one entry
per machine.

The numbers are produced by `bench/ports/run.py`, which builds the programs
in `bench/ports/`, checks that all of them print the same thing, and times
them under two experimental designs. That file is the procedure; this one is
only what came out of it.

To add a machine, run the harness and append its printed block to MACHINES.
Nothing else here has to change: the page renders however many machines the
list holds, and grows a cross-machine comparison as soon as there are two.

Absolute milliseconds are a property of the hardware and do not travel. The
ratio to C mostly does, which is why a second machine is compared to the
first on ratios and never on raw times.
"""

# The four programs, in the order every `ms` and `spread` list uses.
PROGRAMS = [
    {
        "key": "fib", "name": "fib(35)",
        "size_en": "9.2M recursive calls", "size_fr": "9,2 M d'appels récursifs",
        "what_en": "Call overhead and integer arithmetic. Nothing is allocated, so a "
                   "language's memory strategy cannot help or hurt it.",
        "what_fr": "Coût d'appel et arithmétique entière. Rien n'est alloué, donc la "
                   "stratégie mémoire d'un langage ne peut ni l'aider ni le pénaliser.",
    },
    {
        "key": "loops", "name": "loops",
        "size_en": "100M iterations", "size_fr": "100 M d'itérations",
        "what_en": "A while loop, a modulo and an addition. The body compiles to the "
                   "same handful of instructions in every language that compiles.",
        "what_fr": "Une boucle while, un modulo, une addition. Le corps se compile en la "
                   "même poignée d'instructions dans tout langage qui compile.",
    },
    {
        "key": "objects", "name": "objects",
        "size_en": "10M allocations", "size_fr": "10 M d'allocations",
        "what_en": "A two-field record built and dropped inside every iteration. This "
                   "is where reference counting, tracing collection and stack "
                   "allocation give different answers.",
        "what_fr": "Un record à deux champs construit et détruit à chaque itération. "
                   "C'est là que comptage de références, ramasse-miettes et allocation "
                   "sur la pile donnent des réponses différentes.",
    },
    {
        "key": "lists", "name": "lists",
        "size_en": "1M elements through map / filter / fold",
        "size_fr": "1 M d'éléments à travers map / filter / fold",
        "what_en": "Each language uses its own list type and its own higher-order "
                   "functions. It is the least like-for-like of the four, and the "
                   "column where that matters most.",
        "what_fr": "Chaque langage utilise son propre type de liste et ses propres "
                   "fonctions d'ordre supérieur. C'est le moins comparable des quatre, "
                   "et la colonne où cela compte le plus.",
    },
]

# The order languages appear in, everywhere. Grouped by how they reach the
# machine — ahead of time, then on a virtual machine, then interpreted —
# which is a fact about them, not a ranking.
LANGS = ["C", "C++", "Rust", "Keal", "Go", "Java", "Kotlin", "Python"]

# Keal appears once, as `keal build`. Its two interpreters are a different
# question and the page says so rather than quietly leaving them out.
SUBJECT = "Keal"


MACHINES = [
    {
        'key': 'linux-arm64',
        'name_en': 'Linux · aarch64',
        'name_fr': 'Linux · aarch64',
        'cpu_en': '6-core aarch64 guest under QEMU, 7 GB',
        'cpu_fr': 'invité aarch64 6 cœurs sous QEMU, 7 Go',
        'os': 'Ubuntu, Linux 7.0.0',
        'date': '2026-09-04',
        'keal': 'keal 1.2.0',
        'runs': 18,
        'order_effects': 1,
        'toolchains': [
            [
                'C',
                'gcc (Ubuntu 15.2.0-16ubuntu1) 15.2.0',
                '-O2 -std=c11',
            ],
            [
                'C++',
                'g++ (Ubuntu 15.2.0-16ubuntu1) 15.2.0',
                '-O2 -std=c++17',
            ],
            [
                'Rust',
                'rustc 1.98.0 (88d9e12ae 2026-08-18)',
                '-C opt-level=2',
            ],
            [
                'Keal',
                'keal 1.2.0',
                'keal build',
            ],
            [
                'Go',
                'go version go1.25.1 linux/arm64',
                'go build',
            ],
            [
                'Java',
                'openjdk version "25.0.4" 2026-07-21',
                'javac, default JVM',
            ],
            [
                'Kotlin',
                'kotlinc-jvm 2.4.10 (JRE 25.0.4+7-1-26.04-Ubuntu); '
                'jars run on openjdk version "25.0.4" 2026-07-21',
                '-include-runtime, jars run on the PATH java',
            ],
            [
                'Python',
                'CPython 3.14.4',
                'stock build',
            ],
        ],
        'startup': {
            'C': 0.2,
            'C++': 0.3,
            'Rust': 0.4,
            'Keal': 0.3,
            'Go': 0.8,
            'Java': 14.9,
            'Kotlin': 23.2,
            'Python': 6.2,
        },
        'ms': {
            'C': [9.1, 43.1, 10.1, 3.9],
            'C++': [9.3, 43.8, 10.2, 5.8],
            'Rust': [16.1, 40.5, 9.3, 3.5],
            'Keal': [20.4, 44.2, 10.0, 13.4],
            'Go': [22.7, 46.2, 11.1, 30.9],
            'Java': [15.3, 43.9, 14.2, 97.3],
            'Kotlin': [13.1, 42.6, 13.7, 78.7],
            'Python': [449.0, 4683.8, 1464.8, 94.8],
        },
        'spread': {
            'C': [9, 7, 8, 19],
            'C++': [6, 7, 7, 13],
            'Rust': [7, 8, 5, 17],
            'Keal': [7, 3, 7, 4],
            'Go': [3, 5, 5, 89],
            'Java': [6, 9, 6, 11],
            'Kotlin': [11, 10, 12, 8],
            'Python': [8, 3, 5, 7],
        },
    },
    {
        'key': 'macos-arm64',
        'name_en': 'macOS · Apple M4',
        'name_fr': 'macOS · Apple M4',
        'cpu_en': '10-core Apple M4 (4 performance + 6 efficiency), arm64, '
                  'bare metal, 24 GB — the only machine here that is not a guest',
        'cpu_fr': 'Apple M4 10 cœurs (4 performance + 6 efficacité), arm64, '
                  'matériel nu, 24 Go — la seule machine ici qui ne soit pas un invité',
        'os': 'macOS 26.5.1, Darwin 25.5.0',
        'date': '2026-09-04',
        'keal': 'keal 1.2.0',
        'runs': 18,
        'order_effects': 0,
        'toolchains': [
            [
                'C',
                'Apple clang version 21.0.0 (clang-2100.1.1.101)',
                '-O2 -std=c11',
            ],
            [
                'C++',
                'Apple clang version 21.0.0 (clang-2100.1.1.101)',
                '-O2 -std=c++17',
            ],
            [
                'Rust',
                'rustc 1.98.0 (88d9e12ae 2026-08-18)',
                '-C opt-level=2',
            ],
            [
                'Keal',
                'keal 1.2.0',
                'keal build',
            ],
            [
                'Go',
                'go version go1.27.0 darwin/arm64',
                'go build',
            ],
            [
                'Java',
                'openjdk version "25.0.4.1" 2026-08-18',
                'javac, default JVM',
            ],
            [
                'Kotlin',
                'kotlinc-jvm 2.4.10 (JRE 25.0.4.1); jars run on '
                'openjdk version "25.0.4.1" 2026-08-18',
                '-include-runtime, jars run on the PATH java',
            ],
            [
                'Python',
                'CPython 3.14.7',
                'stock build',
            ],
        ],
        'startup': {
            'C': 1.8,
            'C++': 2.0,
            'Rust': 1.9,
            'Keal': 1.7,
            'Go': 2.2,
            'Java': 18.9,
            'Kotlin': 26.3,
            'Python': 15.6,
        },
        'ms': {
            'C': [16.4, 32.5, 6.0, 2.5],
            'C++': [16.4, 32.8, 6.0, 2.9],
            'Rust': [16.2, 33.4, 9.1, 2.9],
            'Keal': [20.9, 45.4, 9.5, 10.4],
            'Go': [20.4, 46.0, 9.7, 7.3],
            'Java': [14.2, 42.7, 12.5, 48.7],
            'Kotlin': [17.0, 41.8, 13.0, 41.5],
            'Python': [605.3, 5860.2, 1687.5, 106.6],
        },
        'spread': {
            'C': [8, 8, 13, 12],
            'C++': [7, 11, 5, 7],
            'Rust': [7, 11, 6, 16],
            'Keal': [9, 2, 5, 10],
            'Go': [8, 5, 12, 27],
            'Java': [21, 8, 8, 10],
            'Kotlin': [10, 14, 7, 12],
            'Python': [8, 8, 11, 21],
        },
    },
]


# ---------------------------------------------------------------- the prose

TEXT = {
    "en": {
        "title": "Eight languages, one machine",
        "lede": "Four small programs, written once per language and checked to print the "
                "same bytes, timed on the same idle machine — with the controls that say "
                "which of the differences are real.",
        "programs_h": "The four programs",
        "programs_p": "They were chosen for four different costs. The most useful result on "
                      "this page is that they do not agree on an ordering.",
        "results_h": "Compute time",
        "results_p": "Every figure is the minimum of the runs minus that runtime's own "
                     "hello-world time, so a JVM's boot is not charged to its arithmetic. "
                     "The minimum rather than the median: a machine under interference "
                     "produces a long right tail and no left one.",
        "ratio_h": "Against C",
        "ratio_p": "Ratios survive a change of machine better than absolute times do, which "
                   "is why a second machine is compared on this table and not on the one "
                   "above. They are not immune: a ratio divides by whatever the local C "
                   "compiler produced, so it carries that compiler's decisions with it. "
                   "On <code>fib</code> that is most of what separates the machines — see "
                   "the limits below.",
        "spread_h": "How solid each figure is",
        "spread_p": "How far the median sits above the minimum, as a percentage. Below "
                    "about 10% a figure is settled; well above it, two languages a few "
                    "percent apart are not ranked by this data, and the page does not "
                    "pretend otherwise.",
        "controls_h": "What was checked before any of this was believed",
        "controls_p": "A benchmark is an instrument, and an instrument nobody calibrated "
                      "reports its own defects as findings. Each of these could have come "
                      "back negative.",
        "limits_h": "What this does not establish",
        "method_h": "Running it yourself",
        "method_p": "The programs are in <code>bench/ports/</code> and the harness that "
                    "times them is <code>bench/ports/run.py</code>. It builds what it finds "
                    "a toolchain for, names what it skips, and prints an entry ready to "
                    "append to the machine list. Every language uses 64-bit signed integers "
                    "and, where it has the choice, the optimisation level "
                    "<code>keal build</code> itself passes to <code>cc</code>.",
        "toolchain_h": "Toolchains",
        "cross_h": "The same ratios, machine by machine",
        "cross_p": "Where two machines disagree about a ratio, the disagreement is the "
                   "result — the number was a property of one box rather than of the "
                   "language. Read the denominator first: several rows moving together "
                   "and in the same direction is the signature of the C baseline having "
                   "changed, not of five languages changing at once.",
        "th_lang": "Language", "th_startup": "startup", "th_machine": "Machine",
        "th_version": "Version", "th_flags": "Build", "th_program": "Program",
        "th_stresses": "What it stresses", "th_size": "Size",
        "alone": "Only one machine has reported so far. A second one turns the ratio "
                 "table into a comparison; the harness in <code>bench/ports/</code> prints "
                 "an entry ready to paste.",
    },
    "fr": {
        "title": "Huit langages, une machine",
        "lede": "Quatre petits programmes, écrits une fois par langage et vérifiés comme "
                "imprimant les mêmes octets, chronométrés sur la même machine au repos — "
                "avec les contrôles qui disent lesquelles des différences sont réelles.",
        "programs_h": "Les quatre programmes",
        "programs_p": "Ils ont été choisis pour quatre coûts différents. Le résultat le plus "
                      "utile de cette page est qu'ils ne s'accordent sur aucun classement.",
        "results_h": "Temps de calcul",
        "results_p": "Chaque chiffre est le minimum des exécutions moins le temps de "
                     "démarrage propre à ce runtime, pour que l'amorçage d'une JVM ne soit "
                     "pas imputé à son arithmétique. Le minimum plutôt que la médiane : une "
                     "machine perturbée produit une longue queue à droite et aucune à gauche.",
        "ratio_h": "Rapporté à C",
        "ratio_p": "Les rapports résistent mieux au changement de machine que les temps "
                   "absolus, et c'est pourquoi une seconde machine se compare sur ce "
                   "tableau-ci et non sur le précédent. Ils n'y sont pas insensibles : un "
                   "rapport divise par ce qu'a produit le compilateur C local, donc il "
                   "emporte les décisions de ce compilateur avec lui. Sur <code>fib</code>, "
                   "c'est l'essentiel de ce qui sépare les machines — voir les limites.",
        "spread_h": "La solidité de chaque chiffre",
        "spread_p": "De combien la médiane dépasse le minimum, en pourcentage. Sous 10% "
                    "environ, un chiffre est acquis ; bien au-dessus, deux langages séparés "
                    "de quelques pour cent ne sont pas départagés par ces données, et la "
                    "page ne prétend pas le contraire.",
        "controls_h": "Ce qui a été vérifié avant de croire quoi que ce soit",
        "controls_p": "Un banc d'essai est un instrument, et un instrument que personne n'a "
                      "étalonné rapporte ses propres défauts comme des résultats. Chacun de "
                      "ces contrôles pouvait revenir négatif.",
        "limits_h": "Ce que cela n'établit pas",
        "method_h": "Le refaire soi-même",
        "method_p": "Les programmes sont dans <code>bench/ports/</code> et le harnais qui "
                    "les chronomètre est <code>bench/ports/run.py</code>. Il construit ce "
                    "pour quoi il trouve une chaîne d'outils, nomme ce qu'il saute, et "
                    "imprime une entrée prête à être ajoutée à la liste des machines. Chaque "
                    "langage utilise des entiers signés 64 bits et, quand il a le choix, le "
                    "niveau d'optimisation que <code>keal build</code> passe lui-même à "
                    "<code>cc</code>.",
        "toolchain_h": "Chaînes d'outils",
        "cross_h": "Les mêmes rapports, machine par machine",
        "cross_p": "Là où deux machines ne s'accordent pas sur un rapport, le désaccord est "
                   "le résultat — le chiffre était une propriété d'une machine et non du "
                   "langage. Lisez le dénominateur d'abord : plusieurs lignes qui bougent "
                   "ensemble et dans le même sens signent un changement de la référence C, "
                   "et non cinq langages changeant à la fois.",
        "th_lang": "Langage", "th_startup": "démarrage", "th_machine": "Machine",
        "th_version": "Version", "th_flags": "Compilation", "th_program": "Programme",
        "th_stresses": "Ce qu'il sollicite", "th_size": "Taille",
        "alone": "Une seule machine a rapporté pour l'instant. Une seconde transforme le "
                 "tableau des rapports en comparaison ; le harnais dans "
                 "<code>bench/ports/</code> imprime une entrée prête à coller.",
    },
}


# Each control is a claim the measurement could have failed. The figure is
# what it actually returned on the machines that have reported.
CONTROLS = [
    ("32 / 32 agree", "32 / 32 concordent",
     "The programs compute the same thing",
     "Les programmes calculent la même chose",
     "Every implementation is run and its last line compared against the Keal "
     "reference before any clock starts. A faster program that prints a different "
     "number is not a faster program.",
     "Chaque implémentation est exécutée et sa dernière ligne comparée à la "
     "référence Keal avant tout chronométrage. Un programme plus rapide qui "
     "imprime un autre nombre n'est pas un programme plus rapide."),

    ("ruled out by scaling", "écarté par mise à l'échelle",
     "No compiler computed the answer at build time",
     "Aucun compilateur n'a calculé la réponse à la compilation",
     "Ten times the work must cost ten times the time. Checked by rebuilding at "
     "10× and re-timing rather than by reading the assembly — though the loop's "
     "backward branch was confirmed there too. Ten times the work came back at "
     "9.0–9.6× the time.",
     "Dix fois le travail doit coûter dix fois le temps. Vérifié en reconstruisant "
     "à ×10 et en rechronométrant plutôt qu'en lisant l'assembleur — bien que la "
     "branche arrière de la boucle y ait aussi été confirmée. Dix fois le travail "
     "est revenu à 9,0–9,6 fois le temps."),

    # `{n}` is filled from the machines themselves, so this sentence cannot
    # drift away from the numbers the way a hand-written one does.
    ("two designs compared", "deux plans comparés",
     "Whether run order moves the numbers",
     "Si l'ordre d'exécution déplace les chiffres",
     "The whole set is measured twice: once blocked, every replicate of a "
     "configuration consecutively, and once interleaved in a shuffled order. The "
     "gap between the two designs is then held against that configuration's own "
     "spread within a single design. {n} — and with a threshold this crude applied "
     "32 times per machine, a marginal crossing or two is what chance alone "
     "produces, so the count is reported rather than interpreted.",
     "L'ensemble est mesuré deux fois : une fois groupé, chaque réplique d'une "
     "configuration d'affilée, une fois entrelacé dans un ordre tiré au sort. "
     "L'écart entre les deux plans est ensuite confronté à la dispersion propre à "
     "cette configuration à l'intérieur d'un seul plan. {n} — et avec un seuil "
     "aussi grossier appliqué 32 fois par machine, un franchissement marginal ou "
     "deux est ce que le hasard seul produit, si bien que le compte est rapporté "
     "plutôt qu'interprété."),

    ("no drift", "aucune dérive",
     "Nothing warmed up or wore out mid-run",
     "Rien ne s'est échauffé ni essoufflé en cours de route",
     "Rank correlation between when a run happened in the shuffled sequence and "
     "how long it took. A machine heating up, or a cache filling, would show as a "
     "consistent sign across configurations. The median was 0.19, well under what "
     "nine replicates make significant.",
     "Corrélation de rang entre le moment d'une exécution dans la séquence "
     "entrelacée et sa durée. Une machine qui chauffe, ou un cache qui se "
     "remplit, apparaîtrait comme un signe constant d'une configuration à "
     "l'autre. La médiane vaut 0,19, bien en deçà du seuil que neuf répliques "
     "rendent significatif."),

    ("one discarded round", "un tour jeté",
     "The page cache was full before the clock started",
     "Le cache de pages était plein avant le départ du chronomètre",
     "The first execution of a binary reads it from disk. Every configuration is "
     "run once and thrown away before measurement, so no language pays for being "
     "first in the sequence.",
     "La première exécution d'un binaire le lit depuis le disque. Chaque "
     "configuration est exécutée une fois et jetée avant la mesure, pour qu'aucun "
     "langage ne paie le fait d'être passé en premier."),

    ("named, not smoothed", "nommées, pas lissées",
     "Configurations too unstable to rank are said so",
     "Les configurations trop instables sont désignées",
     "The spread table is not decoration. Where a configuration's median sits far "
     "above its minimum, its position against a close neighbour is not resolved by "
     "this data, and the page names it instead of picking a winner.",
     "Le tableau de dispersion n'est pas décoratif. Là où la médiane d'une "
     "configuration dépasse largement son minimum, sa position face à un voisin "
     "proche n'est pas tranchée par ces données, et la page le dit au lieu de "
     "désigner un gagnant."),
]


LIMITS = [
    ("<code>lists</code> is not like-for-like.",
     "<code>lists</code> n'est pas comparable à l'identique.",
     "Keal's <code>List&lt;Int&gt;</code>, C++'s <code>vector</code> and Rust's "
     "<code>Vec</code> hold unboxed 64-bit integers; Java and Kotlin box every "
     "element into a <code>Long</code>. That difference is most of what the JVM "
     "pays in that column, and it is a property of the libraries rather than of "
     "the compilers.",
     "Le <code>List&lt;Int&gt;</code> de Keal, le <code>vector</code> de C++ et le "
     "<code>Vec</code> de Rust contiennent des entiers 64 bits non boxés ; Java et "
     "Kotlin boxent chaque élément dans un <code>Long</code>. Cette différence "
     "explique l'essentiel de ce que la JVM paie dans cette colonne, et c'est une "
     "propriété des bibliothèques plutôt que des compilateurs."),

    ("Each timing is a whole process, run once.",
     "Chaque mesure est un processus entier, exécuté une fois.",
     "No warm-up loop inside the program, no steady-state measurement. This is "
     "what running the program costs, which flatters ahead-of-time compilers and "
     "understates what a JIT does in a long-lived server.",
     "Pas de boucle de chauffe dans le programme, pas de mesure en régime établi. "
     "C'est ce que coûte l'exécution du programme, ce qui avantage les "
     "compilateurs anticipés et sous-estime ce qu'un JIT fait dans un serveur de "
     "longue durée."),

    ("Four programs are four programs.",
     "Quatre programmes sont quatre programmes.",
     "They were chosen for four different costs, but no set this small predicts a "
     "real workload, and none of them touches I/O, strings or concurrency.",
     "Ils ont été choisis pour quatre coûts différents, mais aucun ensemble aussi "
     "petit ne prédit une charge réelle, et aucun ne touche aux entrées-sorties, "
     "aux chaînes ni à la concurrence."),

    ("The ratio to C carries the C compiler with it.",
     "Le rapport à C emporte le compilateur C avec lui.",
     "A ratio divides by whatever the local C compiler produced, so it absorbs "
     "how fast the hardware is but not the baseline's own optimisation "
     "decisions. <code>fib</code> is where that shows. On the Linux machine, "
     "gcc at <code>-O2</code> inlines the recursion several levels deep — 244 "
     "instructions in the function body against 30 with inlining turned off — "
     "and the same source, same compiler and same flag then runs in 11.8 ms "
     "against 20.3 ms. That 1.7× swing is about the size of the gap between the "
     "machines' own C baselines on that program. <code>objects</code> is the "
     "second such column and it points the other way: against the clang "
     "machine, six of seven rows move together by about 1.6× because that C "
     "baseline is the fast one there. Between the two gcc machines the same "
     "column stays inside the pack, which is what says the effect belongs to "
     "the compiler rather than to the hardware. So those two ratios compare "
     "within a machine and not across C compilers, and any row that moves "
     "between machines has to be read against its denominator before it is "
     "read as a fact about the language.",
     "Un rapport divise par ce qu'a produit le compilateur C local : il absorbe "
     "la vitesse du matériel, pas les décisions d'optimisation de la référence "
     "elle-même. <code>fib</code> est l'endroit où cela se voit. Sur la machine "
     "Linux, gcc en <code>-O2</code> déroule la récursion sur plusieurs niveaux "
     "— 244 instructions dans le corps de la fonction contre 30 sans inlining — "
     "et la même source, le même compilateur et le même drapeau s'exécutent "
     "alors en 11,8 ms contre 20,3 ms. Ce facteur 1,7 est de l'ordre de l'écart "
     "entre les références C des machines sur ce programme. <code>objects</code> "
     "est la seconde colonne dans ce cas, et elle pointe en sens inverse : face "
     "à la machine sous clang, six lignes sur sept montent ensemble d'environ "
     "1,6× parce que c'est là que la référence C est la rapide. Entre les deux "
     "machines sous gcc, cette même colonne reste dans le peloton — c'est ce "
     "qui dit que l'effet appartient au compilateur et non au matériel. Ces "
     "deux rapports se comparent donc à l'intérieur d'une machine et non d'un "
     "compilateur C à l'autre, et toute ligne qui bouge d'une machine à l'autre "
     "doit être lue contre son dénominateur avant d'être lue comme un fait sur "
     "le langage."),

    ("Subtracting startup weighs most on the rows that have the most of it.",
     "La soustraction du démarrage pèse le plus là où il y en a le plus.",
     "Each figure has its own machine's hello-world time taken off it, which is "
     "what keeps a JVM's boot out of its arithmetic. But a JVM's boot is tens "
     "of milliseconds where a native binary's is under two, so the JVM rows "
     "carry by far the largest correction — and it moves: on the macOS machine, "
     "aligning the JDK changed Java's startup by 8 ms, which is 8 ms on all "
     "four of its figures and a tenth of its <code>lists</code>. Comparing a "
     "JVM row across machines compares two startup corrections as well as two "
     "runtimes.",
     "Chaque chiffre est diminué du temps de hello-world de sa propre machine, "
     "ce qui garde l'amorçage d'une JVM hors de son arithmétique. Mais cet "
     "amorçage se compte en dizaines de millisecondes là où celui d'un binaire "
     "natif tient sous deux : les lignes JVM portent donc de loin la plus "
     "grosse correction — et elle bouge. Sur la machine macOS, aligner le JDK a "
     "déplacé le démarrage de Java de 8 ms, soit 8 ms sur ses quatre chiffres "
     "et un dixième de son <code>lists</code>. Comparer une ligne JVM d'une "
     "machine à l'autre, c'est comparer deux corrections de démarrage autant "
     "que deux runtimes."),

    ("The machines do not share a toolchain.",
     "Les machines ne partagent pas leur chaîne d'outils.",
     "A ratio to C absorbs how fast the hardware is. It does not absorb a "
     "different gcc, a different Go, or a different JVM, and each machine "
     "reports whatever it had. Where the runtime is itself the thing being "
     "measured — the Java and Kotlin rows — the machines align on a JDK major "
     "version, because otherwise a gap between two machines would confound the "
     "operating system with the virtual one. Everywhere else the version is "
     "declared rather than forced, and the toolchain table under each machine "
     "is that disclosure.",
     "Un rapport à C absorbe la vitesse du matériel. Il n'absorbe ni un gcc "
     "différent, ni un Go différent, ni une JVM différente, et chaque machine "
     "rapporte ce qu'elle avait. Là où le runtime est lui-même l'objet de la "
     "mesure — les lignes Java et Kotlin — les machines s'alignent sur une "
     "version majeure de JDK, faute de quoi un écart entre deux machines "
     "confondrait le système d'exploitation et la machine virtuelle. Partout "
     "ailleurs la version est déclarée plutôt qu'imposée, et le tableau des "
     "chaînes d'outils sous chaque machine est cette divulgation."),

    ("A ratio is a property of a machine until a second machine agrees.",
     "Un rapport est une propriété d'une machine tant qu'une seconde ne l'a pas confirmé.",
     "The first machine here failed to reproduce a ratio this project had "
     "published, by a factor of two, for exactly that reason. Until the table "
     "below has more than one column, read every number as measured rather than "
     "as true.",
     "La première machine ici n'a pas reproduit un rapport que ce projet avait "
     "publié, d'un facteur deux, pour exactement cette raison. Tant que le tableau "
     "ci-dessous n'a qu'une colonne, lisez chaque chiffre comme mesuré et non "
     "comme vrai."),
]
