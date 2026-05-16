#!/usr/bin/env bash
set -euo pipefail

# luanti-gate — script de désinstallation
# Supprime le wrapper, la config et l'entrée .desktop. Préserve la base de données.

BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"
CONFIG_DIR="$HOME/.config/luanti-gate"
DESKTOP_FILE="$APPS_DIR/luanti.desktop"
TARGET_BIN="$BIN_DIR/luanti"

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/luanti-gate"

echo "==> Suppression du binaire wrapper..."
rm -f "$TARGET_BIN"

echo "==> Suppression de l'entrée .desktop..."
rm -f "$DESKTOP_FILE"

echo "==> Suppression du dossier de configuration..."
rm -rf "$CONFIG_DIR"

echo "==> Mise à jour de la base de données desktop..."
update-desktop-database "$APPS_DIR" 2>/dev/null || true

echo ""
echo "============================================================"
echo "  Désinstallation terminée."
echo ""
echo "  La base de données (historique des contrôles, questions)"
echo "  a été PRÉSERVÉE à :"
echo "      $DATA_DIR"
echo ""
echo "  Si tu veux vraiment effacer tout l'historique, lance :"
echo "      rm -rf \"$DATA_DIR\""
echo "============================================================"
