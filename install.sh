#!/usr/bin/env bash
# Compile Gwords en mode release et l'installe dans le dossier utilisateur (~/.local).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ID="io.github.gwords.Gwords"
PREFIX="${HOME}/.local"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo introuvable. Sur Silverblue, lancez ce script dans la toolbox de développement :" >&2
    echo "  toolbox run -c gwords-dev ./install.sh" >&2
    exit 1
fi

echo "Compilation (release)..."
(cd "$ROOT" && cargo build --release)

echo "Installation du binaire..."
install -Dm755 "$ROOT/target/release/gwords" "$PREFIX/bin/gwords"

echo "Installation du fichier desktop..."
install -Dm644 "$ROOT/data/$APP_ID.desktop" "$PREFIX/share/applications/$APP_ID.desktop"

echo "Installation des métadonnées AppStream..."
install -Dm644 "$ROOT/data/$APP_ID.metainfo.xml" "$PREFIX/share/metainfo/$APP_ID.metainfo.xml"

echo "Installation des icônes..."
install -Dm644 "$ROOT/data/icons/hicolor/scalable/apps/$APP_ID.svg" \
    "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$ROOT/data/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
    "$PREFIX/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

# Mise à jour des caches, si les outils sont présents.
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -q -t -f "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "Gwords est installé dans $PREFIX/bin/gwords"
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) echo "Attention : $PREFIX/bin n'est pas dans le PATH." ;;
esac
