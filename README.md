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

## Compilation et exécution

Prérequis : Rust 1.85 ou plus récent (édition 2024), GTK 4.20 ou plus récent et libadwaita 1.7 ou plus récent,
avec leurs paquets de développement.

```sh
git clone https://github.com/PremiumCPST/gwords.git
cd gwords
```

Sur Fedora Silverblue, on utilise une toolbox dédiée :

```sh
toolbox create gwords-dev
toolbox enter gwords-dev
sudo dnf install rust cargo gtk4-devel libadwaita-devel gcc
cargo run --release
```

Sur une Fedora classique, installez les mêmes paquets avec `sudo dnf install`, puis lancez `cargo run --release`.

Pour installer l'application dans `~/.local` (binaire, fichier `.desktop`, métadonnées et icônes) :

```sh
toolbox run -c gwords-dev ./install.sh
```

Le script compile la version `release` et copie les fichiers ; il peut aussi être lancé directement
sur un système où `cargo` est disponible.

## Raccourcis clavier

| Action                          | Raccourci            |
|---------------------------------|----------------------|
| Nouvel onglet                   | Ctrl+N ou Ctrl+T     |
| Nouvelle fenêtre                | Ctrl+Maj+N           |
| Ouvrir un document              | Ctrl+O               |
| Enregistrer                     | Ctrl+S               |
| Enregistrer sous                | Ctrl+Maj+S           |
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
| Préférences                     | Ctrl+,               |
| Quitter                         | Ctrl+Q               |

## Licence

Gwords est distribué sous licence GPL-3.0-or-later.
