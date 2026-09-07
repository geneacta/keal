# The Keal family's visual identity

The five marks, at the size every site uses them: 256 pixels, transparent
ground, one file each. They live in the parent's repository because the
identity is the family's and not any one site's — and because a session
working on a child cannot read a file that only exists on somebody else's
machine, which is what held this up for a day.

Fetch one directly:

    curl -O https://raw.githubusercontent.com/geneacta/keal/main/design/logos/kealsql.png

`keal.png` → keal · `kealsql.png` → kealsql · `keal-view.png` → keal-view ·
`kealeb.png` → kealeb · `kealler.png` → the Kealler page of the Keal site.
Each is copied into its site as `assets/k.png` and serves the nav mark, the
tab icon and the share card at once.

The originals are 480–780 pixels and are Tony's; these are `sips -Z 256`
of them, which is what four uses need — a mark drawn at thirty pixels, a
favicon, an `og:image`, and a README banner at 120. The one that was in a
site before this weighed 438 KB and was carried by every page.

What follows is the handoff as written.

---

# Handoff : identité visuelle de la famille Keal

Quatre sites statiques (GitHub Pages, générés par `site/build.py`, une feuille `site/style.css` chacun) et une page à part :

- `geneacta/keal` — le parent, https://geneacta.github.io/keal/
- `geneacta/kealsql` — https://geneacta.github.io/kealsql/
- `geneacta/keal-view` — https://geneacta.github.io/keal-view/
- `geneacta/kealeb` — https://geneacta.github.io/kealeb/
- la page `kealler.html` du site Keal, traitée comme un sous-site.

## Ce que contient ce dossier

- `Famille Keal.dc.html` + `support.js` — la référence de design (planche 1a, maquettes 1b–1d, tokens 1e, page Kealler 2a). **C'est un prototype HTML, pas du code à copier** : il faut appliquer les valeurs ci-dessous dans chaque `site/style.css` et dans les templates de `site/build.py`.
- `logos/*.png` — les cinq K détourés (fond transparent), à copier dans `site/assets/` de chaque dépôt.
- `screenshots/` — rendu de chaque planche : `1a-planche-famille.png`, `1b-kealsql.png`, `1c-keal-view.png`, `1d-kealeb.png`, `1e-tokens.png`, `2a-kealler.png`.
- `github.md` — les dépôts et fichiers sources lus pour construire la référence.

## Fidélité

Haute fidélité pour les couleurs, polices, marques et blocs de code. Les maquettes 1b–1d reprennent la structure de nav / hero / cartes déjà en place dans chaque `index.html` : ne pas la modifier, seulement re-teinter.

## Principe

Keal est le parent et **ne change pas** (`--bg #0B1514`, `--accent #35C8A8`, Sora, JetBrains Mono). Chaque enfant garde le même `style.css` (mêmes noms de variables, mêmes composants, mêmes rayons) et change six choses :

1. la teinte d'accent (même clarté et saturation, teinte OKLCH différente : 170 → 280 / 80 / 20) ;
2. la nuance du noir de fond, teintée vers l'accent ;
3. la police des grands titres (`--display`), le corps reste Sora ;
4. la marque : `K` à gauche du nom + petit signe après le nom ;
5. le motif de fond du hero (`--motif`) ;
6. l'habillage des blocs de code.

## 1. Tokens — remplacer le bloc `:root` de chaque `site/style.css`

### kealsql (teinte 280, pervenche)
```css
:root {
  --bg: #0E0F16; --panel: #17181F;
  --line: rgba(183,189,255,.12); --line2: rgba(183,189,255,.16);
  --ink: #E9EAF5; --dim: #9A9DB1; --prose: #ADAFC4; --faint: #65677A;
  --accent: #A1A7FF; --accent-h: #ADB4FF; --mint: #CCD2FF;
  --kw: #B1B7FD; --ty: #CCD2FF; --str: #D9C98B; --code: #CED0DE;
  --display: 'Space Grotesk', Sora, sans-serif;
  --motif: linear-gradient(rgba(161,167,255,.06) 1px, transparent 1px),
           linear-gradient(90deg, rgba(161,167,255,.06) 1px, transparent 1px);
  --motif-size: 48px 48px;
}
```
Toutes les couleurs `rgba(140,220,196,…)` / `rgba(169,235,205,…)` / `rgba(53,200,168,…)` codées en dur dans le fichier deviennent `rgba(183,189,255,…)` / `rgba(204,210,255,…)` / `rgba(161,167,255,…)` à alpha identique. Le `.btn-gh` (fond `--mint`) devient donc lavande clair sur texte `--bg`.

### keal-view (teinte 80, ambre)
```css
:root {
  --bg: #130F08; --panel: #1C1811;
  --line: rgba(230,189,119,.12); --line2: rgba(230,189,119,.16);
  --ink: #F0EAE0; --dim: #A89D8A; --prose: #BBAF9C; --faint: #726756;
  --accent: #DCA744; --accent-h: #E9B452; --mint: #F3D29B;
  --kw: #E0B771; --ty: #F3D29B; --str: #9FD3B4; --code: #D8D0C3;
  --display: Geist, Sora, sans-serif;
  --motif: radial-gradient(rgba(220,167,68,.20) 1px, transparent 1.2px);
  --motif-size: 12px 12px;
}
```
Le `style.css` actuel de keal-view a des noms de variables un peu différents (`--panel-hi`, `--soft`, `--ok/--warn/--bad`) : garder ces noms, leur donner `--soft: rgba(220,167,68,.12)`, `--soft2: rgba(220,167,68,.20)`, `--panel-hi: #251F16`, `--panel-down: #2E2719`. Statuts `--ok #34d399 / --warn #fbbf24 / --bad #f87171` inchangés. Les chaînes de caractères (`--str`) passent en vert d'eau pour ne pas se confondre avec l'accent ambre. Le CTA « GitHub » plein passe de `#fff` sur bleu à `--bg` sur `--accent`. Le `.wordmark` masqué sur `assets/keal-view.png` est remplacé par le wordmark texte (§3).

### kealeb (teinte 20, corail)
```css
:root {
  --bg: #150D0D; --panel: #1F1615;
  --line: rgba(254,170,169,.12); --line2: rgba(254,170,169,.16);
  --ink: #F5E7E7; --dim: #B19797; --prose: #C4AAA9; --faint: #796262;
  --accent: #F98E8F; --accent-h: #FF9A9B; --mint: #FFC3C2;
  --kw: #F8A4A3; --ty: #FFC3C2; --str: #D9C98B; --code: #DECCCB;
  --display: 'Familjen Grotesk', Sora, sans-serif;
  --motif: repeating-linear-gradient(-45deg, rgba(249,142,143,.07) 0 1px, transparent 1px 16px);
  --motif-size: auto;
}
```

### Règles communes aux trois enfants (à ajouter en fin de fichier)
```css
.hero h1, .duo h2, .gs-head h2, .band h1, .band h2, .doc-head h1, .prose h1 { font-family: var(--display); }
.hero { background-image: var(--motif); background-size: var(--motif-size); }
```
Google Fonts à ajouter au `<head>` : `Space Grotesk:wght@500;700` (kealsql), `Geist:wght@500;700` (keal-view), `Familjen+Grotesk:wght@500;700` (kealeb), en plus de Sora et JetBrains Mono déjà chargés.

## 2. Contraste

Toutes les couleurs `--dim` / `--prose` sur `--bg` restent ≥ 4,5:1 ; `--accent` sur `--bg` ≥ 7:1 ; `--bg` sur `--accent` (texte des boutons pleins) ≥ 7:1. Ne pas éclaircir les fonds.

## 3. Marque : logo K + wordmark texte

Pour les quatre sites (Keal abandonne `assets/keal.png`) :

```html
<a class="mark" href="index.html">
  <img class="mark-k" src="assets/k.png" alt="">
  <span class="wordmark">keal<span class="suffix">sql</span></span>
</a>
```
```css
.mark { display: flex; align-items: center; gap: 9px; }
.mark-k { display: block; width: 30px; height: 30px; object-fit: contain; }
.wordmark { font: 700 22px Sora, sans-serif; letter-spacing: -.02em; color: var(--ink); }
.wordmark .suffix { color: var(--accent); font-weight: 500; }
.wordmark::after { content: ""; display: inline-block; width: 7px; height: 7px;
                   margin-left: 7px; vertical-align: 3px; }
```
Le signe après le nom, par site :
- keal — `width: 14px; height: 7px; border-radius: 99px; background: var(--accent);` (le pied du K, suffixe vide)
- kealsql — `border: 2px solid var(--accent); box-sizing: border-box;` (cellule ouverte)
- keal-view — `background: var(--accent);` (pixel plein, pas de rayon)
- kealeb — `background: var(--accent); border-radius: 50%;` (le point déjà en place)
- kealler — suffixe `ler` en `#A1A7FF`, signe `width:14px; height:7px; border-radius:99px; background: linear-gradient(90deg, #35C8A8, #A1A7FF);`

Fichiers : `logos/keal.png` → keal, `logos/kealsql.png` → kealsql, `logos/keal-view.png` → keal-view, `logos/kealeb.png` → kealeb, `logos/kealler.png` → page Kealler. Renommer en `assets/k.png` dans chaque dépôt (et `assets/k-kealler.png` dans keal). Ce sont des PNG ~480–780 px, fond transparent ; le même fichier sert de favicon :

```html
<link rel="icon" type="image/png" href="assets/k.png">
```
(Un export 32×32 et 180×180 depuis le PNG suffit si l'on veut `apple-touch-icon`.)

Le bouton « Keal » dans la nav des enfants garde un petit pied menthe `#35C8A8` (10×5 px, arrondi) devant le mot : le seul vert autorisé chez un enfant.

## 4. Blocs de code — un habillage par site

Tous : `background: var(--panel); border: 1px solid var(--line2); border-radius: 14px` (12 px dans les docs), `pre` en JetBrains Mono 13–13,5 px / 1.75, couleur `--code`.

- **keal** (inchangé) — barre avec trois points `rgba(140,220,196,.18)` + nom de fichier.
- **kealsql** — barre d'onglets : l'onglet actif en `--ink` avec `border-bottom: 2px solid var(--accent)`, le second (`▸ sql`) en `--faint`. Sous le code, un second `pre` séparé par `border-top: 1px solid var(--line)` sur fond `rgba(161,167,255,.05)`, couleur `--dim`, mots-clés SQL en `--kw`, commentaires en `--faint`. Sur le hero, ce bloc a `white-space: pre-wrap`.
- **keal-view** — barre : nom de fichier à gauche, taille de la fenêtre (`320 × 200 pt`) à droite en `--faint`. Sous le code, une zone `background: var(--bg); padding: 18px 22px` contenant la fenêtre rendue : `background: var(--panel); border: 1px solid rgba(230,189,119,.3); border-radius: 4px` (angles serrés = pixel), bouton `--accent` texte `--bg` rayon 4 px. Légende 12 px `--faint` dessous.
- **kealeb** — barre en mono 12 px : `GET` en `--accent`, chemin en `--code`, à droite `200 · one patch` en `--faint`. Sous le code, `border-top: 1px dashed rgba(254,170,169,.3)`, le patch JSON en `--mint` 13 px, une ligne d'explication en Sora 12,5 px `--dim`.

## 5. Heros (maquettes 1b–1d)

Structure existante : grille 1fr 1fr, gap 56 px, padding 72 px 40 px 64 px. Pill : mono 600 12 px, couleur `--mint`, bordure `rgba(<mint>,.25)`, rayon 99 px, padding 6 px 14 px, marge basse 26 px ; sa puce (6 px) prend la forme du signe du site (rond / carré). H1 : `var(--display)` 700, 54 px (56 px pour Geist) / 1.1, letter-spacing −.02 à −.03 em. Sous-titre Sora 18 px / 1.65 `--dim`, max 52 ch. CTA plein : Sora 600 15 px, `--bg` sur `--accent`, rayon 10 px, padding 13 px 24 px ; CTA contour : bordure `rgba(<line>,.25)`. Textes exacts : ceux des `index.html` actuels.

## 6. Page Kealler (2a)

Vit dans le site Keal, même `style.css`, avec un `<body class="kealler">` qui surcharge :
```css
.kealler { --accent: #A1A7FF; --accent-h: #ADB4FF; --mint: #CCD2FF; }
.kealler .hero { background: linear-gradient(135deg, rgba(53,200,168,.07), transparent 45%, rgba(161,167,255,.07)); }
```
Le code reste coloré en menthe (`--kw #63D3B4`, `--ty #A9EBCD`) ; l'accent pervenche sert aux boutons, à l'onglet actif, aux erreurs (soulignement `underline wavy #A1A7FF`, pastille « ● 1 error »). Logo `k-kealler.png`, wordmark `keal` + `ler`. Fenêtre IDE du hero : barre trois points, colonne fichiers 150 px (fichier actif `background: rgba(161,167,255,.12); border-left: 2px solid #A1A7FF`), barre de statut mono 11 px `--faint`.

## 7. Ce qui ne change pas

Sora + JetBrains Mono pour le corps et le code ; nav (padding 16 px 40 px, liens 500 14 px gap 26 px) ; cartes (`--panel`, bordure `--line`, rayon 14 px, padding 24 px) ; footers ; grilles docs / tour ; points de rupture 900–980 px.
