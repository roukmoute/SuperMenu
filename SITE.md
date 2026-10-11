# Site de présentation

Le site statique publié sur https://roukmoute.github.io/SuperMenu/ se trouve dans `site/`. Il n'utilise ni framework, ni compilation, ni police distante, ni outil de suivi.

## Modifier et vérifier

Modifier `site/index.html`, `site/styles.css` et `site/script.js`. Les captures dans `site/assets/` sont des fichiers locaux du projet. Pour vérifier le rendu depuis la racine du dépôt :

```powershell
python -m http.server 8765 --bind 127.0.0.1 --directory site
```

Ouvrir http://127.0.0.1:8765/ dans un navigateur. Vérifier le français, l'anglais, les quatre aperçus, les questions fréquentes et un écran étroit. Le contenu principal et le téléchargement fonctionnent aussi sans JavaScript.

## Publication GitHub Pages

Choisir une fois **Settings → Pages → Build and deployment → Source → GitHub Actions** dans le dépôt. Le workflow `.github/workflows/pages.yml` publie ensuite `site/` à chaque modification du site sur `main`. Il peut aussi être lancé manuellement dans Actions. Aucun jeton personnel supplémentaire n'est enregistré dans le projet.

Les liens de téléchargement ciblent une version explicite, car les préversions ne sont pas renvoyées par `releases/latest`. Lors d'une nouvelle version, actualiser le numéro, les liens de téléchargement et les mesures dans les fichiers du site. Les mesures doivent rester liées à leur version et à leur protocole ; une latence du moteur de recherche ne mesure pas le démarrage de l'application.
