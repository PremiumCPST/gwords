# Gwords

Gwords est un traitement de texte pour GNOME, écrit en Rust avec GTK 4 et libadwaita.
Son interface suit les recommandations GNOME (il s'inspire de GNOME Text Editor), et ses animations
accompagnent la frappe, la mise en forme et l'ouverture des documents.

## Fonctionnalités

- Édition de plusieurs documents dans des onglets
- Ouverture et enregistrement des formats OpenDocument (`.odt`), Word (`.docx`) et texte brut (`.txt`)
- Mise en forme : gras, italique, souligné, barré, titres, indices et exposants, alignements
- Listes à puces et listes numérotées
- Recherche et remplacement
- Préférences : animations de frappe et du curseur, police, taille du texte, barre d'état,
  thème clair ou sombre
- Intégration au bureau GNOME : fichier `.desktop`, métadonnées AppStream et icônes

### Mode avancé

Le mode avancé ajoute une seconde barre d'outils sous la barre principale. Pour l'activer ou le
désactiver, ouvrez le menu principal et cochez **Mode avancé** (raccourci `Ctrl+Maj+A`). Il propose :

- **Tableaux** : insertion, ajout et suppression de lignes et de colonnes ; chaque cellule accepte
  du texte mis en forme. Les tableaux sont lus et écrits en `.odt`, `.docx` et `.txt` (en texte brut,
  chaque ligne du tableau devient une ligne séparée par des tabulations).
- **Caractères spéciaux** : insertion de symboles et de caractères typographiques.
- **Liens hypertexte** : ajout ou modification d'un lien sur le texte sélectionné (`Ctrl+K`).
- **Changement de casse** : majuscules, minuscules, initiale de chaque mot ou début de phrase.
- **Date et heure** : insertion de la date et de l'heure courantes.
- **Statistiques du document** : nombre de mots, de caractères, de paragraphes et de tableaux,
  et durée de lecture estimée.

## Installation

Gwords est distribué uniquement en **Flatpak**, ce qui le rend utilisable sur toutes les distributions
Linux qui prennent en charge Flatpak. Il s'appuie sur le runtime GNOME 51 de Flathub.

1. Téléchargez `gwords.flatpak` depuis la [dernière release](https://github.com/PremiumCPST/gwords/releases/latest).
2. Installez-le (le runtime GNOME est récupéré automatiquement depuis Flathub) :

```sh
flatpak install --user gwords.flatpak
flatpak run io.github.gwords.Gwords
```

Gwords apparaît ensuite dans le menu des applications. L'application n'a aucun accès direct à vos
fichiers : l'ouverture et l'enregistrement passent par les portails (les dialogues de fichiers du système).

## Compilation depuis les sources

Prérequis : `flatpak` et le dépôt Flathub.

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.flatpak.Builder org.gnome.Sdk//51 org.gnome.Platform//51 \
    org.freedesktop.Sdk.Extension.rust-stable//26.08

git clone https://github.com/PremiumCPST/gwords.git
cd gwords
flatpak run org.flatpak.Builder --user --install --force-clean build-dir io.github.gwords.Gwords.json
flatpak run io.github.gwords.Gwords
```

Pour produire un paquet `gwords.flatpak` distribuable :

```sh
flatpak run org.flatpak.Builder --user --force-clean --repo=repo build-dir io.github.gwords.Gwords.json
flatpak build-bundle repo gwords.flatpak io.github.gwords.Gwords \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
```

### Dépendances Rust

La construction Flatpak se fait hors ligne : les crates sont décrites dans `build-aux/cargo-sources.json`.
Après toute modification de `Cargo.lock`, régénérez ce fichier avec
[flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo) :

```sh
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
```

Pour développer sans Flatpak (avec Rust, GTK 4.20+ et libadwaita 1.7+ installés), `cargo run` et
`cargo test` fonctionnent directement.

## Raccourcis clavier

| Action                          | Raccourci            |
|---------------------------------|----------------------|
| Nouvel onglet                   | Ctrl+N ou Ctrl+T     |
| Nouvelle fenêtre                | Ctrl+Maj+N           |
| Ouvrir un document              | Ctrl+O               |
| Enregistrer                     | Ctrl+S               |
| Enregistrer sous                | Ctrl+Maj+S           |
| Mode avancé                     | Ctrl+Maj+A           |
| Fermer l'onglet                 | Ctrl+W               |
| Rechercher                      | Ctrl+F               |
| Rechercher et remplacer         | Ctrl+H               |
| Gras                            | Ctrl+B               |
| Italique                        | Ctrl+I               |
| Souligné                        | Ctrl+U               |
| Barré                           | Ctrl+Maj+X           |
| Titre (0 = texte normal, 1 à 3) | Ctrl+0 à Ctrl+3      |
| Aligner à gauche                | Ctrl+L               |
| Centrer                         | Ctrl+E               |
| Aligner à droite                | Ctrl+R               |
| Justifier                       | Ctrl+J               |
| Liste à puces                   | Ctrl+Maj+8           |
| Liste numérotée                 | Ctrl+Maj+7           |
| Effacer la mise en forme        | Ctrl+M               |
| Lien hypertexte                 | Ctrl+K               |
| Préférences                     | Ctrl+,               |
| Quitter                         | Ctrl+Q               |

## Licence

Gwords est distribué sous licence GPL-3.0-or-later.
