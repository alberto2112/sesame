#!/usr/bin/env bash
#
# Sésame — installation (Arch / Manjaro, KDE Plasma 6, Wayland).
#
# Compile, installe les quatre binaires, déclare une session SDDM, et propose
# (sans jamais l'imposer) de fermer les portes de sortie.
#
# À lancer en utilisateur normal, PAS en root : le script demande sudo quand il
# en a besoin, et seulement là.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_DIR=/usr/local/bin
SESSION_DIR=/usr/share/wayland-sessions
LOGIND_DROPIN=/etc/systemd/logind.conf.d/50-sesame.conf

# Sésame a vécu en X11 jusqu'à Plasma 5. Les machines qui viennent de cette
# époque gardent une entrée de session périmée : elle lance `startplasma-x11`,
# qui n'existe plus. On la retire, sinon SDDM propose une session morte.
OLD_SESSION_X11=/usr/share/xsessions/sesame.desktop
OLD_XORG_DROPIN=/etc/X11/xorg.conf.d/50-sesame.conf

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

# La porte tient sur trois pièces. On les vérifie AVANT de compiler quoi que ce
# soit : découvrir qu'il manque `cage` après le premier redémarrage, c'est
# découvrir un enfant devant un écran noir.
say "Vérification de la plateforme…"

command -v cage >/dev/null \
    || die "« cage » est introuvable. C'est le compositeur qui porte le contrôle.
    Installe-le :  sudo pacman -S cage"
echo "    cage           : $(command -v cage)"

command -v startplasma-wayland >/dev/null \
    || die "« startplasma-wayland » est introuvable : pas de bureau à ouvrir.
    Installe Plasma :  sudo pacman -S plasma-desktop"
echo "    Plasma Wayland : $(command -v startplasma-wayland)"

[[ -d /usr/share/wayland-sessions ]] \
    || die "/usr/share/wayland-sessions n'existe pas : SDDM n'attend aucune
    session Wayland sur cette machine."
echo "    Sessions       : $SESSION_DIR"

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

# L'entrée X11 d'avant. Si on la laisse, SDDM propose DEUX sessions « Sésame »,
# dont une qui mène à un écran noir : elle exécute `startplasma-x11`, absent de
# Plasma 6 (la session X11 y est un paquet séparé, `plasma-x11-session`).
if [[ -f "$OLD_SESSION_X11" ]]; then
    say "Retrait de l'ancienne session X11 (périmée)…"
    sudo rm -f "$OLD_SESSION_X11"
    echo "    Supprimée : $OLD_SESSION_X11"
fi
if [[ -f "$OLD_XORG_DROPIN" ]]; then
    # DontZap : « Ctrl+Alt+Retour arrière ne tue plus le serveur X ». Sous
    # Wayland il n'y a pas de serveur X, et ce raccourci n'existe pas — le
    # réglage ne protège plus rien, il traîne.
    sudo rm -f "$OLD_XORG_DROPIN"
    echo "    Supprimé  : $OLD_XORG_DROPIN (DontZap — sans objet sous Wayland)"
fi

# ===== Configuration ========================================================

if [[ -f "$CONFIG_DIR/config.toml" ]]; then
    say "Configuration existante préservée : $CONFIG_DIR/config.toml"
else
    say "Configuration par défaut → $CONFIG_DIR/config.toml"
    install -Dm0644 config.toml "$CONFIG_DIR/config.toml"
fi

# L'import N'EST PAS idempotent : `sesame import` réinsère les questions (INSERT
# simple), et le dédoublonnage qui suit garde la copie la PLUS RÉCENTE — celle du
# JSON. Sur une réinstallation, cela ÉCRASE ton travail : une difficulté corrigée
# à la main est remplacée par celle de la banque, une question supprimée
# réapparaît. On ne réimporte donc QUE si tu le demandes ; seule une base absente
# déclenche l'import d'office (première installation).
#
# Où est cette base ? On ne la devine pas et on ne code pas son chemin en dur :
# on lit `paths.database` dans config.toml — LA source de vérité, celle que lit
# le binaire — et on la résout pareil (chemin relatif => $XDG_DATA_HOME/sesame/).
# Pas de config.toml = rien à préserver = première installation, on prend le
# défaut « questions.db ».
db_rel=""
if [[ -f "$CONFIG_DIR/config.toml" ]]; then
    db_rel=$(sed -n 's/^[[:space:]]*database[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
             "$CONFIG_DIR/config.toml" | head -1)
fi
db_rel="${db_rel:-questions.db}"
case "$db_rel" in
    /*) DB_FILE="$db_rel" ;;                                        # chemin absolu : tel quel
    *)  DB_FILE="${XDG_DATA_HOME:-$HOME/.local/share}/sesame/$db_rel" ;;  # relatif : sous les données
esac

import_questions=yes
if [[ -s "$DB_FILE" ]]; then
    say "Base existante : $DB_FILE"
    echo "    Réimporter réinsère les banques data/*.json et ANNULE tes retouches :"
    echo "    les questions supprimées réapparaissent, les difficultés éditées à la"
    echo "    main sont écrasées par celles du JSON. Réponds « o » UNIQUEMENT si tu"
    echo "    as ajouté de nouvelles banques à importer."
    ask "Réimporter les questions ?" || import_questions=no
fi

if [[ "$import_questions" == yes ]]; then
    # `data/*.json` et non `data/questions_*.json` : les banques importées
    # (data/import_*.json) sont arrivées après, et le motif d'origine les
    # laissait dehors — des milliers de questions qui n'atteignaient jamais la
    # machine.
    say "Import des questions…"
    for f in data/*.json; do
        [[ -e "$f" ]] || continue
        echo "    $f"
        "$BIN_DIR/sesame" import "$f" >/dev/null || warn "échec sur $f"
    done

    # Deux banques finissent toujours par se recouper. On nettoie tout de suite :
    # sans ça, un enfant peut recevoir deux fois le même énoncé au même contrôle.
    say "Suppression des doublons…"
    "$BIN_DIR/sesame" dedupe | tail -1
else
    say "Import ignoré — base et retouches préservées."
fi

# ===== Durcissement (facultatif) ============================================

say "Fermer la porte de sortie ? (facultatif, réversible)"
cat <<'EOF'
    Il n'en reste qu'une, et c'est celle qu'un enfant curieux finit par
    trouver :

      * Ctrl+Alt+F2 … F6  → une console en texte, hors de la session.
        Remède : ne plus faire apparaître de console sur ces touches
        (NAutoVTs=0). Le changement d'écran reste possible, mais il n'y a
        plus rien à y trouver.

    Ce qu'on ne fait PAS : interdire le changement d'écran. Ça t'enfermerait
    TOI aussi, le jour où l'affichage se fige, sans aucune console de secours.
    On laisse toujours une sortie à l'adulte.

    Ce qui a DISPARU en passant à Wayland : Ctrl+Alt+Retour arrière tuait le
    serveur X d'un coup, et il fallait le désarmer (DontZap). Sous Wayland il
    n'y a pas de serveur X à tuer, et cage ne lie ce raccourci à rien. Le
    problème n'est pas corrigé : il n'existe plus.
EOF

if ask "Appliquer ce réglage ?"; then
    sudo install -Dm0644 /dev/stdin "$LOGIND_DROPIN" <<'EOF'
# Sésame — pas de console en texte derrière Ctrl+Alt+F2…F6.
# Supprimer ce fichier annule le réglage.
[Login]
NAutoVTs=0
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
  Session SDDM  : $SESSION_DIR/sesame.desktop   (Wayland)
  Compositeur   : $(command -v cage)
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
