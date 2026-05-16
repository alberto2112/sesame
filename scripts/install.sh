#!/usr/bin/env bash
set -euo pipefail

# luanti-gate — script d'installation (Arch Linux)
# Compile en release, déploie dans ~/.local et enregistre l'entrée .desktop.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"
CONFIG_DIR="$HOME/.config/luanti-gate"
DESKTOP_FILE="$APPS_DIR/luanti.desktop"
TARGET_BIN="$BIN_DIR/luanti"
REAL_GAME="/usr/bin/luanti"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "Erreur : ce script ne fonctionne que sur Linux (Arch). Système détecté : $(uname -s)." >&2
    exit 1
fi

if [[ ! -x "$REAL_GAME" ]]; then
    echo "Erreur : le binaire réel de Luanti est introuvable à $REAL_GAME." >&2
    echo "Installe-le d'abord : sudo pacman -S luanti" >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Erreur : cargo n'est pas installé. Installe Rust : sudo pacman -S rust" >&2
    exit 1
fi

echo "==> Compilation en mode release..."
cargo build --release

echo "==> Création des dossiers..."
mkdir -p "$BIN_DIR" "$APPS_DIR" "$CONFIG_DIR"

echo "==> Installation du binaire dans $TARGET_BIN..."
install -m 0755 "$REPO_ROOT/target/release/luanti" "$TARGET_BIN"

if [[ -f "$CONFIG_DIR/config.toml" ]]; then
    echo "==> Config existante préservée : $CONFIG_DIR/config.toml"
else
    echo "==> Copie du config par défaut dans $CONFIG_DIR/config.toml..."
    install -m 0644 "$REPO_ROOT/config.toml" "$CONFIG_DIR/config.toml"
fi

echo "==> Écriture de l'entrée .desktop..."
cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Luanti
GenericName=Voxel Game (gated)
Exec=$HOME/.local/bin/luanti %f
Icon=luanti
Terminal=false
Categories=Game;
StartupNotify=true
EOF
chmod 0644 "$DESKTOP_FILE"

echo "==> Mise à jour de la base de données desktop..."
update-desktop-database "$APPS_DIR" 2>/dev/null || true

echo "==> Importation des questions d'exemple..."
"$TARGET_BIN" import "$REPO_ROOT/data/samples/example.json" || true

case ":$PATH:" in
    *":$BIN_DIR:"*)
        ;;
    *)
        echo ""
        echo "Attention : $BIN_DIR n'est pas dans ton PATH."
        echo "Ajoute cette ligne à ton ~/.bashrc ou ~/.zshrc :"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

echo ""
echo "============================================================"
echo "  Installation terminée avec succès."
echo "  - Binaire          : $TARGET_BIN"
echo "  - Configuration    : $CONFIG_DIR/config.toml"
echo "  - Entrée desktop   : $DESKTOP_FILE"
echo ""
echo "  Lance le jeu depuis ton menu d'applications (Luanti) ou"
echo "  exécute simplement : luanti"
echo "============================================================"
