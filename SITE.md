# Presentation website

**English** · [Français](SITE.fr.md)

The static website published at [roukmoute.github.io/SuperMenu](https://roukmoute.github.io/SuperMenu/?lang=en) uses the files in `site/` for its landing page. The documentation pages are built from the repository's Markdown files. It uses no framework, remote font or tracking tool.

English documentation is available under `/SuperMenu/documentation/`; French documentation is under `/SuperMenu/fr/documentation/`. Every page offers the corresponding page in the other language, including matching section anchors. The landing page's documentation links follow the selected language.

## Editing and checking

Edit `site/index.html`, `site/styles.css` and `site/script.js` for the landing page. Captures in `site/assets/` are local project files. For documentation, edit the English default and its French counterpart together: `README.md` / `README.fr.md`, `TESTING.md` / `TESTING.fr.md`, `PERFORMANCE.md` / `PERFORMANCE.fr.md`, `SITE.md` / `SITE.fr.md`, `locales/README.md` / `locales/README.fr.md`, and `THIRD_PARTY_NOTICES.md` / `THIRD_PARTY_NOTICES.fr.md`. Keep matching heading levels and order, and preserve technical identifiers, commands and historical measurements. Original third-party license text remains verbatim in its published language.

The documentation generator requires Python 3.12 or later. Build and check the complete site from the repository root:

```powershell
python -m pip install --require-hashes -r tools/requirements.txt
python tools/build_site.py --output _site
python tools/check_site.py _site
python -m unittest discover -s tests/site -v
node tests/site/landing.cjs
python -m http.server 8765 --bind 127.0.0.1 --directory _site
```

The generated `_site/` directory contains the landing page, both documentation editions and their local assets. Preview this generated directory so that the landing page's language-specific documentation links are populated. GitHub Actions uses Python 3.13 and Node 24 for these checks.

The `styles.css` and `script.js` references in `site/index.html` carry a content fingerprint (`?v=<first 12 characters of SHA-256>`). After changing those files, update the fingerprint to prevent browsers from keeping an old version of styles or translations.

Open [http://127.0.0.1:8765/](http://127.0.0.1:8765/) in a browser. Check French, English, the five previews (applications, Steam games, files, timer, calculator), frequently asked questions and a narrow screen. Check documentation links, tables of contents and language switching on the same section. The main content and download also work without JavaScript.

## GitHub Pages publication

Once, choose **Settings → Pages → Build and deployment → Source → GitHub Actions** in the repository. The `.github/workflows/pages.yml` workflow then builds and publishes `_site/` when the landing page, documentation or build tools change on `main`. It can also be run manually in Actions. No additional personal token is stored in the project.

Download links target an explicit version because `releases/latest` does not return prereleases. For a new application version, update its number, download links and measurements in the site files. Measurements must remain tied to their version and protocol; search-engine latency does not measure application startup. A documentation-only publication does not change the measured application version or its existing release files.
