# Deploying the site to GitHub Pages

Two steps, once:

1. Copy `pages.yml` into `.github/workflows/pages.yml` and push it.
   (It has to be pushed by credentials with the `workflow` scope — the
   GitHub web UI's "Add file" works fine.)
2. Repository **Settings → Pages → Source: GitHub Actions**.

Every later push touching `site/` redeploys automatically. To refresh
the standard-library page after a prelude change:

```sh
python3 site/build-stdlib.py && git add site/stdlib.html
```
