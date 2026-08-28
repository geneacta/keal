# Handoff : Site web du langage Keal

## Vue d'ensemble
Site vitrine du langage de programmation **Keal** (https://github.com/geneacta/keal) : une landing page, une page « Tour of Keal » (tutoriel guidé) et une page Docs/Référence. Direction retenue : « clair-obscur éditorial » — thème sombre sarcelle, esprit Kotlin Tour, identité propre dérivée de la marque Geneacta.

## À propos des fichiers de design
Les fichiers de ce bundle sont des **références de design réalisées en HTML** (prototype `Keal Site.dc.html`, à ouvrir pour lecture du markup et des styles inline — les cartes des options `2a`, `2b`, `2c` sont les maquettes finales ; la section « turn 1 » contient d'anciennes explorations `1a/1b/1c` à ignorer sauf mention). La tâche est de **recréer ces designs dans l'environnement du site réel** — si aucun n'existe encore, un générateur de site statique (Astro recommandé pour un site de langage : contenu Markdown, zéro JS par défaut) ou Next.js conviennent. Ne pas livrer le HTML tel quel.

## Fidélité
**Haute fidélité (hifi)** : couleurs, typographie, espacements et contenus sont définitifs. Reproduire au pixel près.

## Design tokens

### Couleurs (thème sombre unique)
- Fond page : `#0B1514`
- Fond panneau / carte / fenêtre de code : `#0F1D1B`
- Bordures : `rgba(140,220,196,.12)` (séparateurs) et `rgba(140,220,196,.16)` (fenêtres de code)
- Texte principal : `#E4F1EC`
- Texte secondaire : `#8CA69F` (landing) / `#9FB8B1` (prose tuto/docs)
- Texte discret / prompts `$` : `#567870`
- Accent principal (teal) : `#35C8A8` — CTA pleins, indicateurs actifs, labels de section
- Accent menthe : `#A9EBCD` — CTA secondaire plein (bouton GitHub), sorties de code, inline-code coloré
- Coloration syntaxique : mots-clés `#63D3B4`, types `#A9EBCD`, chaînes `#D9C98B`, commentaires `#567870`, code par défaut `#C7DAD4`

### Typographie
- Titres & UI : **Sora** (Google Fonts) — 400/600/700
- Code : **JetBrains Mono** — 400/500/700
- H1 landing : 700 54px/1.12, letter-spacing -.02em
- H2 sections : 700 34px/1.2, -.015em ; H1 docs : 700 36px ; H2 tuto : 700 32px
- Prose : 400 15.5px/1.75 ; sous-titre hero : 400 18px/1.65
- Labels de section (eyebrow) : JetBrains Mono 600 12px, letter-spacing .08em, couleur accent, MAJUSCULES
- Code blocs : 13–13.5px/1.75–1.8

### Rayons, ombres, espacements
- Rayons : 14px (cartes/fenêtres code), 12px (blocs code tuto/docs), 10px (boutons, champ commande), 8px (petits boutons), 99px (pilules, toggle langue)
- Ombre fenêtre code hero : `0 24px 60px rgba(0,0,0,.45)`
- Gouttières : padding horizontal page 40px ; sections 64px vertical ; gap grilles 18px (cartes) / 56px (2 colonnes)
- Largeur de maquette : 1240px

## Écrans

### 1. Landing (option 2a)
- **Nav** (padding 16px 40px, bordure basse) : logo (img, h 36px) + liens Docs (actif `#E4F1EC`) / Tour / Playground / Blog (14px Sora 500, inactifs `#8CA69F`) ; à droite : toggle EN|FR (pilule, segment actif fond accent texte sombre), badge `v0.5.0` (mono, bordure), bouton GitHub (fond `#A9EBCD`, texte `#0B1514`).
- **Hero** (grid 1fr 1fr, gap 56px, padding 72/40/64) :
  - Gauche : badge pilule mono « Self-hosting — the bootstrap fixed point is verified on every run » avec point accent 6px ; H1 **« The language that compiles itself. »** (ne jamais utiliser « small »/« petit ») ; sous-titre ; 2 CTA (« Start the tour → » plein accent, « Read the docs » contour) ; ligne de commande copiable `$ git clone geneacta/keal && ./bootstrap.sh` avec icône ⧉.
  - Droite : fenêtre de code « point.keal » (3 pastilles, titre mono), snippet class Point + when (voir prototype), pied « $ keal point.keal → (3.0, 4.0) has length 5.0 ».
- **4 cartes features** (grid 4 col) : SELF-HOSTED / NULL SAFETY / INTEROP (6 chips C, C++, Rust, Go, Java, Kotlin) / REPL (mini extrait). Eyebrow accent + titre 16px + prose 13.5px.
- **Section Interop** (2 col, bordure haute) : texte à gauche (H2 « One file, six languages. »), fenêtre `polyglot.keal` à droite (6 println commentés).
- **Section Trois moteurs** (2 col inversées) : carte avec 3 barres de perf (tree-walker 6.14s 100 %, bytecode VM 2.51s 41 %, native via C 0.03s 2.5 % en accent) + légende « fib(35)… » ; texte H2 « ×84 native — with the same guarantees. »
- **Section Getting started** : H2 « Up and running in a minute. » + lien « Then take the tour… → » ; 3 cartes terminal (GET IT / WRITE A FILE / RUN IT).
- **Footer** (grid 2fr 1fr 1fr 1fr) : logo + baseline « A statically typed, self-hosting programming language. Built by Geneacta. » + badge version ; colonnes LEARN / LANGUAGE / PROJECT.

### 2. Tour (option 2b) — gabarit de chapitre
- **Barre haute** : logo (h 30px), « / Tour of Keal », à droite compteur « 6 / 12 », barre de progression 180×5px (dégradé `#35C8A8→#A9EBCD`, remplie à 50 %), bouton « Exit the tour ».
- **Layout** grid 264px + contenu (max 760px, padding 44/56).
- **Sidebar** : label THE TOUR ; 12 chapitres — faits : pastille 20px fond `rgba(53,200,168,.18)` avec ✓ accent ; actif : fond `rgba(53,200,168,.1)` + bordure gauche 2px accent + pastille pleine accent chiffre sombre ; à venir : pastille contour chiffre `#567870`. Encart bas « EVERY SNIPPET RUNS ».
- **Contenu** : eyebrow « CHAPTER 6 · 4 MIN » ; H2 ; prose ; bloc de code avec en-tête (nom de fichier + bouton « ▶ Run » fond menthe) ; 2e bloc sans en-tête ; encart ✦ (fond `rgba(169,235,205,.06)`, bordure `.18`) ; navigation bas : « ← when » (contour) / « Collections & lambdas → » (plein accent).
- Chapitres : Hello world · Values & bindings · fun and proc · Control flow · when · Null safety · Collections & lambdas · Records & classes · Generics & traits · The eight connectives · Errors & diagnostics · Native code & C. Contenus sources : `TUTORIAL.md` du repo (chaque snippet doit être réel et vérifié).

### 3. Docs / Référence (option 2c) — gabarit d'article
- **Nav** : logo + Docs (actif : bordure basse 2px accent) / Tour / Playground / Blog ; champ recherche 260px « ⌕ Search the docs » avec raccourci ⌘K.
- **Layout** grid 264px sidebar + article (max 720px, padding 40/52) + 200px TOC.
- **Sidebar** : groupes GUIDE / LANGUAGE / INTERNALS (labels mono 11px `#567870`), items 13.5px, actif = fond `rgba(53,200,168,.1)` + bordure gauche accent + gras.
- **Article** : fil d'Ariane mono (« Docs / Internals / Memory model », dernier segment menthe) ; H1 ; prose ; bloc code `$ keal layout point.keal` avec bouton ⧉ copy ; H2 « Nullable niches » ; tableau 3 colonnes (TYPE / SIZE / WHY, en-tête fond `rgba(53,200,168,.06)`, lignes séparées par bordure `.1`) ; encart ✦ ; prev/next en contour.
- **TOC droite** : « ON THIS PAGE », items 12.5px sur bordure gauche 1px, actif menthe avec bordure 2px accent.

## Interactions & comportements
- Prototype statique : les états ci-dessous sont à implémenter.
- Liens nav : inactif `#8CA69F` → hover `#E4F1EC` ; actif selon page (couleur pleine ou bordure basse accent).
- CTA plein accent : hover ≈ `#4ADBB5` (éclaircir légèrement) ; CTA contour : hover bordure `rgba(140,220,196,.45)`.
- Ligne de commande et bouton ⧉ : copier dans le presse-papier + feedback « copied ».
- Toggle EN|FR : bascule la langue de tout le site (contenu bilingue prévu ; maquettes en anglais).
- Tour : la progression (chapitres faits) persiste (localStorage) ; « ▶ Run » exécute le snippet et affiche la sortie sous le bloc (voir pied de la fenêtre hero comme modèle de sortie) ; prev/next en bas de chaque chapitre.
- Recherche docs : ⌘K ouvre une palette de recherche.
- Barres de perf : peuvent s'animer à l'entrée dans le viewport (width 0 → valeur, ~600ms ease-out).

## Gestion d'état
- `lang: "en" | "fr"` (toggle nav, persisté)
- `tourProgress: number[]` chapitres complétés (persisté)
- Chapitre courant / article courant : par routing (`/tour/null-safety`, `/docs/memory-model`)
- Sortie d'exécution par bloc runnable : `idle | running | done(output)`

## Contenus
Tous les textes et snippets proviennent du repo (README.md, TUTORIAL.md, docs/). Les reprendre verbatim — chaque snippet du tour est vérifié par la suite de tests du langage. Interdit : qualifier le langage de « small »/« petit ».

## Assets
- `keal.png` — logo Keal fourni par le client (PNG transparent 499×300, à poser tel quel sur fond sombre ; hauteurs d'usage : 36px nav landing, 34px footer, 30–32px barres internes). Prévoir un export SVG à terme.
- Fonts : Sora + JetBrains Mono via Google Fonts (`display=swap`).
- Le logo Geneacta (marque mère) n'est pas utilisé dans les pages, seulement la palette qui en dérive.

## Fichiers
- `Keal Site.dc.html` — prototype HTML (styles inline = source de vérité des valeurs) ; options `2a` (landing), `2b` (tour), `2c` (docs) dans la section `id="t2"`.
- `keal.png` — logo.
