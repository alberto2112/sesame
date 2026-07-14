#!/usr/bin/env bash
#
# Sésame — installation (Arch / Manjaro, KDE Plasma 5, X11).
#
# Compile, installe les trois binaires, déclare une session SDDM, et propose
# (sans jamais l'imposer) de fermer les portes de sortie.
#
# À lancer en utilisateur normal, PAS en root : le script demande sudo quand il
# en a besoin, et seulement là.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_DIR=/usr/local/bin
SESSION_DIR=/usr/share/xsessions
LOGIND_DROPIN=/etc/systemd/logind.conf.d/50-sesame.conf
XORG_DROPIN=/etc/X11/xorg.conf.d/50-sesame.conf

say()  { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m/!\\\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mErreur :\033[0m %s\n' "$*" >&2; exit 1; }

ask() {  # ask "question" -> 0 si oui
    local answer
    read -rp "$(printf '\033[1;35m?\033[0m %s [o/N] ' "$1")" answer
    [[ "$answer" =~ ^[oOyY]$ ]]
}

# ===== Vérifications ========================================================

[[ "$(uname -s)" == Linux ]] || die "ce script ne fonctionne que sous Linux."
[[ $EUID -ne 0 ]] || die "ne lance pas ce script en root. Lance-le normalement : ./scripts/install.sh"
command -v cargo >/dev/null || die "cargo est introuvable. Installe Rust : sudo pacman -S rust"

KID_USER="$USER"
say "Compte des enfants : $KID_USER"
echo "    C'est le compte Linux où la session « Sésame » sera proposée."
echo "    Les profils des enfants (Zoé, Hugo…) vivent DANS l'application, pas"
echo "    dans des comptes Linux séparés : un seul compte suffit."
ask "Est-ce le bon compte ?" || die "relance le script depuis le compte des enfants."

CONFIG_DIR="$HOME/.config/sesame"

# ===== Compilation ==========================================================

say "Compilation en mode release…"
cargo build --release

for bin in sesame sesame-kiosk sesame-timer; do
    [[ -x "target/release/$bin" ]] || die "binaire manquant : target/release/$bin"
done

# ===== Navigateur ===========================================================

say "Recherche d'un navigateur pour le kiosque…"
BROWSER_FOUND=""
for b in chromium chromium-browser google-chrome-stable brave firefox; do
    if command -v "$b" >/dev/null; then BROWSER_FOUND="$b"; break; fi
done
if [[ -n "$BROWSER_FOUND" ]]; then
    echo "    Trouvé : $BROWSER_FOUND"
else
    warn "Aucun navigateur graphique trouvé."
    echo "    Le kiosque n'aurait rien pour afficher le contrôle."
    echo "    Installe-en un :  sudo pacman -S chromium"
    ask "Continuer quand même ?" || exit 1
fi

# ===== Installation système =================================================

say "Installation des binaires dans $BIN_DIR (sudo)…"
sudo install -Dm0755 target/release/sesame       "$BIN_DIR/sesame"
sudo install -Dm0755 target/release/sesame-kiosk "$BIN_DIR/sesame-kiosk"
sudo install -Dm0755 target/release/sesame-timer "$BIN_DIR/sesame-timer"
sudo install -Dm0755 scripts/sesame-session      "$BIN_DIR/sesame-session"

say "Déclaration de la session SDDM dans $SESSION_DIR…"
sudo install -Dm0644 scripts/sesame.desktop "$SESSION_DIR/sesame.desktop"

# ===== Configuration ========================================================

if [[ -f "$CONFIG_DIR/config.toml" ]]; then
    say "Configuration existante préservée : $CONFIG_DIR/config.toml"
else
    say "Configuration par défaut → $CONFIG_DIR/config.toml"
    install -Dm0644 config.toml "$CONFIG_DIR/config.toml"
fi

say "Import des questions…"
for f in data/questions_*.json; do
    [[ -e "$f" ]] || continue
    echo "    $f"
    "$BIN_DIR/sesame" import "$f" >/dev/null || warn "échec sur $f"
done

# ===== Durcissement (facultatif) ============================================

say "Fermer les portes de sortie ? (facultatif, réversible)"
cat <<'EOF'
    Un enfant curieux finit par trouver ces deux-là :

      * Ctrl+Alt+F2 … F6  → une console en texte, hors de la session.
        Remède : ne plus faire apparaître de console sur ces touches
        (NAutoVTs=0). Le changement d'écran reste possible, mais il n'y a
        plus rien à y trouver.

      * Ctrl+Alt+Retour arrière  → tue le serveur X d'un coup.
        Remède : DontZap.

    Ce qu'on ne fait PAS : interdire le changement d'écran (DontVTSwitch).
    Ça t'enfermerait TOI aussi, le jour où l'affichage se fige, sans aucune
    console de secours. On laisse toujours une sortie à l'adulte.
EOF

if ask "Appliquer ces deux réglages ?"; then
    sudo install -Dm0644 /dev/stdin "$LOGIND_DROPIN" <<'EOF'
# Sésame — pas de console en texte derrière Ctrl+Alt+F2…F6.
# Supprimer ce fichier annule le réglage.
[Login]
NAutoVTs=0
EOF
    sudo install -Dm0644 /dev/stdin "$XORG_DROPIN" <<'EOF'
# Sésame — Ctrl+Alt+Retour arrière ne tue plus le serveur X.
# Supprimer ce fichier annule le réglage.
Section "ServerFlags"
    Option "DontZap" "true"
EndSection
EOF
    echo "    Écrit. Actif au prochain démarrage."
else
    echo "    Ignoré. Tu pourras toujours le faire plus tard."
fi

# ===== Le compte des enfants doit être sans pouvoirs ========================

if id -nG "$KID_USER" | grep -qw wheel; then
    warn "$KID_USER appartient au groupe « wheel » : il peut utiliser sudo."
    echo "    Avec sudo, un enfant défait tout ça en une commande."
    echo "    À corriger depuis un AUTRE compte administrateur :"
    echo "        sudo gpasswd -d $KID_USER wheel"
fi

# ===== Et voilà =============================================================

cat <<EOF

============================================================
  Sésame est installé.

  Binaires      : $BIN_DIR/sesame{,-kiosk,-timer,-session}
  Session SDDM  : $SESSION_DIR/sesame.desktop
  Configuration : $CONFIG_DIR/config.toml

  IL RESTE DEUX CHOSES À FAIRE :

  1. Créer le mot de passe administrateur et régler les enfants :

         sesame admin

     Puis, dans le navigateur : un profil par enfant, avec sa difficulté,
     son budget et ses horaires.

  2. Choisir la session « Sésame (contrôle) » au prochain écran de
     connexion.

     Pour qu'elle se lance toute seule — un enfant de 6 ans ne choisit pas
     une session — crée /etc/sddm.conf.d/autologin.conf :

         [Autologin]
         User=$KID_USER
         Session=sesame

  Désinstallation :  ./scripts/uninstall.sh
============================================================

EOF
