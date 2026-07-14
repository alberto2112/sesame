#!/usr/bin/env bash
#
# Sésame — désinstallation.
#
# Retire les binaires, la session SDDM et le durcissement. La BASE DE DONNÉES
# est préservée : elle contient les questions et l'historique pédagogique des
# enfants. On ne détruit jamais ça sans qu'on nous le demande deux fois.

set -euo pipefail

BIN_DIR=/usr/local/bin
SESSION_DIR=/usr/share/xsessions
LOGIND_DROPIN=/etc/systemd/logind.conf.d/50-sesame.conf
XORG_DROPIN=/etc/X11/xorg.conf.d/50-sesame.conf

CONFIG_DIR="$HOME/.config/sesame"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/sesame"

say() { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }

say "Suppression des binaires (sudo)…"
sudo rm -f "$BIN_DIR"/sesame "$BIN_DIR"/sesame-kiosk \
           "$BIN_DIR"/sesame-timer "$BIN_DIR"/sesame-session

say "Suppression de la session SDDM…"
sudo rm -f "$SESSION_DIR/sesame.desktop"

say "Retrait du durcissement…"
sudo rm -f "$LOGIND_DROPIN" "$XORG_DROPIN"
echo "    Les consoles texte et Ctrl+Alt+Retour arrière reviendront au"
echo "    prochain démarrage."

say "Suppression de la configuration…"
rm -rf "$CONFIG_DIR"

cat <<EOF

============================================================
  Désinstallation terminée.

  /!\\ Si l'autologin pointe encore sur la session « sesame », le
      prochain démarrage n'aura plus de session à lancer. Vérifie :

          sudo cat /etc/sddm.conf.d/autologin.conf

      et remets Session=plasma (ou supprime le fichier).

  La BASE DE DONNÉES a été PRÉSERVÉE :

      $DATA_DIR

  Elle contient les questions et tout l'historique des contrôles.
  Pour l'effacer vraiment :

      rm -rf "$DATA_DIR"
============================================================

EOF
