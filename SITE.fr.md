# Site de présentation

[English](SITE.md) · **Français**

Le site statique publié sur [roukmoute.github.io/SuperMenu](https://roukmoute.github.io/SuperMenu/?lang=fr) utilise les fichiers de `site/` pour sa page de présentation. Les pages de documentation sont construites à partir des fichiers Markdown du dépôt. Il n'utilise ni framework, ni police distante, ni outil de suivi.

La documentation anglaise est accessible sous `/SuperMenu/documentation/` ; la documentation française sous `/SuperMenu/fr/documentation/`. Chaque page propose la page correspondante dans l'autre langue, avec les ancres de section associées. Les liens de documentation de la page de présentation suivent la langue sélectionnée.

## Modifier et vérifier

Modifier `site/index.html`, `site/styles.css` et `site/script.js` pour la page de présentation. Les captures dans `site/assets/` sont des fichiers locaux du projet. Pour la documentation, modifier ensemble la version anglaise par défaut et son équivalent français : `README.md` / `README.fr.md`, `TESTING.md` / `TESTING.fr.md`, `PERFORMANCE.md` / `PERFORMANCE.fr.md`, `SITE.md` / `SITE.fr.md`, `locales/README.md` / `locales/README.fr.md` et `THIRD_PARTY_NOTICES.md` / `THIRD_PARTY_NOTICES.fr.md`. Conserver les niveaux et l'ordre des titres correspondants, ainsi que les identifiants techniques, commandes et mesures historiques. Les textes originaux des licences tierces restent reproduits à l'identique dans leur langue de publication.

Le générateur de documentation nécessite Python 3.12 ou ultérieur. Construire et vérifier le site complet depuis la racine du dépôt :

```powershell
python -m pip install --require-hashes -r tools/requirements.txt
python tools/build_site.py --output _site
python tools/check_site.py _site
python -m unittest discover -s tests/site -v
node tests/site/landing.cjs
python -m http.server 8765 --bind 127.0.0.1 --directory _site
```

Le dossier généré `_site/` contient la page de présentation, les deux éditions de la documentation et leurs ressources locales. Prévisualiser ce dossier généré pour que les liens de documentation par langue soient renseignés dans la page de présentation. GitHub Actions utilise Python 3.13 et Node 24 pour ces vérifications.

Les références à `styles.css` et `script.js` dans `site/index.html` portent une empreinte de contenu (`?v=<12 premiers caractères du SHA-256>`). Après modification de ces fichiers, actualiser cette empreinte pour éviter qu'un navigateur conserve une ancienne version des styles ou des traductions.

Ouvrir [http://127.0.0.1:8765/](http://127.0.0.1:8765/) dans un navigateur. Vérifier le français, l'anglais, les cinq aperçus (applications, jeux Steam, fichiers, minuteur, calculatrice), les questions fréquentes et un écran étroit. Vérifier les liens de documentation, les sommaires et le changement de langue sur une même section. Le contenu principal et le téléchargement fonctionnent aussi sans JavaScript.

## Publication GitHub Pages

Choisir une fois **Settings → Pages → Build and deployment → Source → GitHub Actions** dans le dépôt. Le workflow `.github/workflows/pages.yml` construit et publie ensuite `_site/` lorsque la page de présentation, la documentation ou les outils de construction changent sur `main`. Il peut aussi être lancé manuellement dans Actions. Aucun jeton personnel supplémentaire n'est enregistré dans le projet.

Les liens de téléchargement ciblent une version explicite, car les préversions ne sont pas renvoyées par `releases/latest`. Lors d'une nouvelle version de l'application, actualiser son numéro, les liens de téléchargement et les mesures dans les fichiers du site. Les mesures doivent rester liées à leur version et à leur protocole ; une latence du moteur de recherche ne mesure pas le démarrage de l'application. Une publication de documentation seule ne change ni la version de l'application mesurée ni les fichiers de sa release existante.
