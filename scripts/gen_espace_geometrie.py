#!/usr/bin/env python3
"""Génère la banque « Espace et Géométrie » : des dessins à reproduire.

Deux types, ceux que l'enfant fait à l'école :
  * grid_cells — des CASES coloriées forment une figure ;
  * grid_lines — des SEGMENTS relient les points du treillis (horizontaux,
    verticaux et diagonaux à 45°).

## Deux partis pris

**Les figures ne sont pas aléatoires.** Un semis de cases au hasard est plus dur
à recopier qu'une forme cohérente, et n'apprend rien : ce qu'on travaille, c'est
le repérage — « la case est deux rangs plus bas et un cran à droite » —, pas la
mémoire photographique. D'où un catalogue de formes reconnaissables : lettres,
cadres, escaliers, croix, maisons, flèches, losanges, sapins.

**Chaque forme est déclinée dans ses HUIT orientations** (quatre rotations, avec
et sans miroir). Ce n'est pas un artifice pour gonfler les chiffres : c'est
l'exercice lui-même. Un L couché n'est pas un L debout pour l'enfant qui a du mal
à se repérer — c'est précisément le muscle qu'on veut faire travailler. Et une
forme symétrique (une croix, un damier) se replie toute seule sur elle-même : la
déduplication s'en occupe, on ne produit jamais deux fois le même dessin.

Sortie au format d'import de sesame, sous forme CANONIQUE (jetons triés,
extrémités ordonnées) — la même que produit `grid::Grid::serialize` côté Rust.

    python3 scripts/gen_espace_geometrie.py
    cargo run --bin sesame -- import data/questions_espace_geometrie.json
"""
import json
import random
import sys
from collections import Counter

SUBJECT = "Espace et Géométrie"
OUT = "data/questions_espace_geometrie.json"

# Les trois tailles demandées : 4×4 pour commencer, 8×8 comme sur les fiches.
SIZES = (4, 6, 8)

# Combien de calibres de boîte par forme et par taille (une petite, une grande),
# et combien d'orientations retenues pour chacun. 2 × 3 = six dessins par forme
# et par taille — assez pour que l'enfant ne retombe pas sur le même, pas assez
# pour noyer la liste du panel admin.
CALIBRES = 2
ORIENTATIONS = 3

# Graine fixe : relancer le script doit redonner exactement la même banque,
# sinon un réimport créerait des centaines de questions de plus au lieu de
# retrouver les siennes.
RNG = random.Random(20260725)

CONSEILS = [
    "Repère-toi en comptant les cases depuis le bord.",
    "Commence par un coin, puis avance case par case.",
    "Vérifie chaque rangée du modèle avant de valider.",
    "Regarde d'abord la forme entière, puis les détails.",
    "Le dessin est peut-être tourné : regarde bien dans quel sens.",
]

# ===== Formes en CASES ======================================================
# Chaque fonction reçoit une boîte de bh × bw cases et rend les cases occupées,
# en coordonnées locales (ligne, colonne). L'orientation n'a pas d'importance
# ici : les huit variantes sont fabriquées plus bas.


def f_cadre(bh, bw):
    return {(r, c) for r in range(bh) for c in range(bw)
            if r in (0, bh - 1) or c in (0, bw - 1)}


def f_plein(bh, bw):
    return {(r, c) for r in range(bh) for c in range(bw)}


def f_L(bh, bw):
    return {(r, 0) for r in range(bh)} | {(bh - 1, c) for c in range(bw)}


def f_T(bh, bw):
    return {(0, c) for c in range(bw)} | {(r, bw // 2) for r in range(bh)}


def f_U(bh, bw):
    return ({(r, 0) for r in range(bh)} | {(r, bw - 1) for r in range(bh)}
            | {(bh - 1, c) for c in range(bw)})


def f_H(bh, bw):
    return ({(r, 0) for r in range(bh)} | {(r, bw - 1) for r in range(bh)}
            | {(bh // 2, c) for c in range(bw)})


def f_E(bh, bw):
    return ({(r, 0) for r in range(bh)} | {(0, c) for c in range(bw)}
            | {(bh // 2, c) for c in range(bw)} | {(bh - 1, c) for c in range(bw)})


def f_F(bh, bw):
    return ({(r, 0) for r in range(bh)} | {(0, c) for c in range(bw)}
            | {(bh // 2, c) for c in range(bw)})


def f_C(bh, bw):
    return ({(r, 0) for r in range(bh)} | {(0, c) for c in range(bw)}
            | {(bh - 1, c) for c in range(bw)})


def f_I(bh, bw):
    return ({(0, c) for c in range(bw)} | {(bh - 1, c) for c in range(bw)}
            | {(r, bw // 2) for r in range(bh)})


def f_S(bh, bw):
    m = bh // 2
    return ({(0, c) for c in range(bw)} | {(m, c) for c in range(bw)}
            | {(bh - 1, c) for c in range(bw)}
            | {(r, 0) for r in range(m)} | {(r, bw - 1) for r in range(m, bh)})


def f_Z(bh, bw):
    k = min(bh, bw)
    return ({(0, c) for c in range(k)} | {(k - 1, c) for c in range(k)}
            | {(i, k - 1 - i) for i in range(k)})


def f_N(bh, bw):
    k = min(bh, bw)
    return ({(r, 0) for r in range(k)} | {(r, k - 1) for r in range(k)}
            | {(i, i) for i in range(k)})


def f_croix(bh, bw):
    return {(bh // 2, c) for c in range(bw)} | {(r, bw // 2) for r in range(bh)}


def f_escalier(bh, bw):
    out = set()
    for i in range(min(bh, bw)):
        out.add((bh - 1 - i, i))
        if i + 1 < bw:
            out.add((bh - 1 - i, i + 1))
    return out


def f_diagonale(bh, bw):
    return {(i, i) for i in range(min(bh, bw))}


def f_chevron(bh, bw):
    mid = bw // 2
    out = set()
    for i in range(min(bh, mid + 1)):
        out.add((i, mid - i))
        out.add((i, mid + i))
    return {(r, c) for (r, c) in out if 0 <= r < bh and 0 <= c < bw}


def f_damier(bh, bw):
    return {(r, c) for r in range(bh) for c in range(bw) if (r + c) % 2 == 0}


def f_coins(bh, bw):
    return {(0, 0), (0, bw - 1), (bh - 1, 0), (bh - 1, bw - 1)}


def f_sablier(bh, bw):
    out = set()
    for r in range(bh):
        k = min(r, bh - 1 - r)
        out.add((r, k))
        out.add((r, bw - 1 - k))
    return out


def f_anneaux(bh, bw):
    """Des carrés concentriques : très lisible, et coûteux à recopier."""
    return {(r, c) for r in range(bh) for c in range(bw)
            if min(r, c, bh - 1 - r, bw - 1 - c) % 2 == 0}


def f_barres(bh, bw):
    return {(r, c) for r in range(bh) for c in range(bw) if r % 2 == 0}


def f_peigne(bh, bw):
    return {(0, c) for c in range(bw)} | {(r, c) for r in range(bh)
                                          for c in range(0, bw, 2)}


def f_triangle(bh, bw):
    k = min(bh, bw)
    return {(r, c) for r in range(k) for c in range(k) if c <= r}


def f_losange(bh, bw):
    k = min(bh, bw)
    if k % 2 == 0:
        k -= 1
    if k < 3:
        return set()
    m = k // 2
    return {(r, c) for r in range(k) for c in range(k)
            if abs(r - m) + abs(c - m) == m}


def f_zigzag(bh, bw):
    out, r = set(), 0
    for c in range(bw):
        out.add((r, c))
        r = bh - 1 if r == 0 else 0
        if c + 1 < bw:
            for rr in range(min(r, bh - 1 - r), max(r, bh - 1 - r) + 1):
                out.add((rr, c))
    return {(r, c) for (r, c) in out if 0 <= r < bh}


def f_fleche(bh, bw):
    """Une hampe et une pointe — la forme dont l'orientation saute aux yeux."""
    m = bw // 2
    out = {(r, m) for r in range(bh)}
    for i in range(min(m + 1, bh)):
        out.add((i, m - i))
        out.add((i, m + i))
    return {(r, c) for (r, c) in out if 0 <= r < bh and 0 <= c < bw}


FORMES_CASES = [
    ("cadre", f_cadre, 3, 3),
    ("plein", f_plein, 2, 2),
    ("L", f_L, 3, 2),
    ("T", f_T, 3, 3),
    ("U", f_U, 3, 3),
    ("H", f_H, 3, 3),
    ("E", f_E, 3, 3),
    ("F", f_F, 3, 3),
    ("C", f_C, 3, 3),
    ("I", f_I, 3, 3),
    ("S", f_S, 3, 3),
    ("Z", f_Z, 3, 3),
    ("N", f_N, 3, 3),
    ("croix", f_croix, 3, 3),
    ("escalier", f_escalier, 3, 3),
    ("diagonale", f_diagonale, 3, 3),
    ("chevron", f_chevron, 3, 3),
    ("damier", f_damier, 2, 2),
    ("coins", f_coins, 3, 3),
    ("sablier", f_sablier, 3, 3),
    ("anneaux", f_anneaux, 3, 3),
    ("barres", f_barres, 3, 2),
    ("peigne", f_peigne, 3, 3),
    ("triangle", f_triangle, 3, 3),
    ("losange", f_losange, 3, 3),
    ("zigzag", f_zigzag, 3, 3),
    ("flèche", f_fleche, 3, 3),
]

# ===== Formes en SEGMENTS ===================================================
# Chaque fonction rend une liste de CHEMINS ; un chemin est une suite de points
# du treillis, et `chemin_en_aretes` la découpe en arêtes d'une case. Tout pas
# doit être horizontal, vertical ou diagonal à 45° — d'où le `k = min(bh, bw)`
# des formes qui portent une diagonale : elles se rabotent en carré.


def s_boite(bh, bw):
    return [[(0, 0), (0, bw), (bh, bw), (bh, 0), (0, 0)]]


def s_boite_croisee(bh, bw):
    k = min(bh, bw)
    return s_boite(k, k) + [[(0, 0), (k, k)], [(k, 0), (0, k)]]


def s_double_boite(bh, bw):
    k = min(bh, bw)
    if k < 3:
        return []
    return s_boite(k, k) + [[(1, 1), (1, k - 1), (k - 1, k - 1), (k - 1, 1), (1, 1)]]


def s_triangle(bh, bw):
    k = min(bh, bw)
    return [[(0, 0), (k, 0), (k, k), (0, 0)]]


def s_losange(bh, bw):
    k = min(bh, bw)
    if k % 2:
        k -= 1
    m = k // 2
    return [[(0, m), (m, k), (k, m), (m, 0), (0, m)]]


def s_maison(bh, bw):
    k = min(bh, bw)
    if k % 2:
        k -= 1
    m = k // 2
    # Un carré, et un toit en deux diagonales qui se rejoignent au sommet.
    return [[(m, 0), (k, 0), (k, k), (m, k)], [(m, 0), (0, m), (m, k)]]


def s_toit(bh, bw):
    k = min(bh, bw)
    if k % 2:
        k -= 1
    m = k // 2
    return [[(m, 0), (0, m), (m, k)]]


def s_sapin(bh, bw):
    """Un tronc et deux étages de branches.

    Les deux chevrons partent du même sommet décalé d'un cran vers le bas, et
    descendent à 45° — c'est la seule pente que le treillis autorise, donc leur
    largeur est dictée par leur hauteur.
    """
    k = min(bh, bw)
    k -= k % 2
    if k < 4:
        return []
    m = k // 2
    return [[(k, m), (m, m)],                        # le tronc
            [(m, 0), (0, m), (m, k)],                # étage haut
            [(k - 2, 1), (m - 1, m), (k - 2, k - 1)]]  # étage bas


def s_zigzag(bh, bw):
    pts, r = [(0, 0)], 0
    for c in range(1, bw + 1):
        r = bh if r == 0 else 0
        pts.append((r, c))
    return [pts]


def s_creneaux(bh, bw):
    """Des créneaux de château : monte, avance, descend, avance."""
    pts, r = [(bh, 0)], bh
    for c in range(bw):
        r = 0 if r == bh else bh
        pts.append((r, c))
        pts.append((r, c + 1))
    return [pts]


def s_escalier(bh, bw):
    pts, r, c = [(bh, 0)], bh, 0
    while r > 0 and c < bw:
        r -= 1
        pts.append((r, c))
        c += 1
        pts.append((r, c))
    return [pts]


def s_fleche(bh, bw):
    m = bh // 2
    return [[(m, 0), (m, bw)], [(0, bw - m), (m, bw), (bh, bw - m)]]


def s_croix(bh, bw):
    return [[(bh // 2, 0), (bh // 2, bw)], [(0, bw // 2), (bh, bw // 2)]]


def s_etoile(bh, bw):
    """Croix et X superposés : huit branches depuis un même centre."""
    k = min(bh, bw)
    if k % 2:
        k -= 1
    if k < 2:
        return []
    m = k // 2
    return [[(m, 0), (m, k)], [(0, m), (k, m)], [(0, 0), (k, k)], [(k, 0), (0, k)]]


def s_X(bh, bw):
    k = min(bh, bw)
    return [[(0, 0), (k, k)], [(k, 0), (0, k)]]


def s_V(bh, bw):
    k = min(bh, bw)
    if k % 2:
        k -= 1
    if k < 2:
        return []
    m = k // 2
    return [[(0, 0), (m, m), (0, k)]]


def s_M(bh, bw):
    k = min(bh, bw)
    if k % 2:
        k -= 1
    if k < 2:
        return []
    m = k // 2
    return [[(k, 0), (0, 0), (m, m), (0, k), (k, k)]]


def s_N(bh, bw):
    k = min(bh, bw)
    return [[(k, 0), (0, 0)], [(0, 0), (k, k)], [(k, k), (0, k)]]


def s_Z(bh, bw):
    k = min(bh, bw)
    return [[(0, 0), (0, k), (k, 0), (k, k)]]


def s_drapeau(bh, bw):
    return [[(0, 0), (bh, 0)], [(0, 0), (0, bw), (bh // 2, 0)]]


def s_fanion(bh, bw):
    """Un mât et un triangle accroché en haut."""
    k = min(bh, bw)
    if k < 2:
        return []
    m = k // 2
    return [[(k, 0), (0, 0)], [(0, 0), (m, m), (m, 0)]]


def s_eclair(bh, bw):
    m = bh // 2
    return [[(0, bw), (m, 0), (m, bw), (bh, 0)]]


def s_papillon(bh, bw):
    """Deux triangles qui se touchent par la pointe."""
    k = min(bh, bw)
    if k % 2:
        k -= 1
    if k < 2:
        return []
    m = k // 2
    return [[(0, 0), (m, m), (k, 0), (0, 0)], [(0, k), (m, m), (k, k), (0, k)]]


def s_cerf_volant(bh, bw):
    """Un losange et sa ficelle.

    Un vrai cerf-volant a la pointe basse plus longue que les autres côtés — et
    c'est justement ce que le treillis interdit : à 45°, la hauteur d'un côté
    dicte sa largeur, si bien que les quatre côtés sont forcément égaux. Ce
    serait donc un losange de plus. On lui attache une ficelle verticale : la
    silhouette redevient distincte, et tous les pas restent traçables.
    """
    k = min(bh, bw)
    k -= k % 2
    if k < 4:
        return []
    m = k // 2
    return [[(0, m), (m, k), (k, m), (m, 0), (0, m)], [(k, m), (k + 2, m)]]


FORMES_LIGNES = [
    ("boîte", s_boite, 2, 2),
    ("boîte croisée", s_boite_croisee, 2, 2),
    ("double boîte", s_double_boite, 3, 3),
    ("triangle", s_triangle, 2, 2),
    ("losange", s_losange, 2, 2),
    ("maison", s_maison, 2, 2),
    ("toit", s_toit, 2, 2),
    ("sapin", s_sapin, 4, 4),
    ("zigzag", s_zigzag, 1, 2),
    ("créneaux", s_creneaux, 1, 2),
    ("escalier", s_escalier, 2, 2),
    ("flèche", s_fleche, 2, 2),
    ("croix", s_croix, 2, 2),
    ("étoile", s_etoile, 2, 2),
    ("X", s_X, 2, 2),
    ("V", s_V, 2, 2),
    ("M", s_M, 2, 2),
    ("N", s_N, 2, 2),
    ("Z", s_Z, 2, 2),
    ("drapeau", s_drapeau, 2, 2),
    ("fanion", s_fanion, 2, 2),
    ("éclair", s_eclair, 2, 2),
    ("papillon", s_papillon, 2, 2),
    ("cerf-volant", s_cerf_volant, 4, 4),
]


# ===== Canonisation — la même règle qu'en Rust ==============================


def norm_arete(a, b):
    """Extrémités triées : tracer de droite à gauche donne le même trait."""
    return (a, b) if a <= b else (b, a)


def chemin_en_aretes(points):
    """Découpe un chemin en arêtes d'UNE case.

    C'est ici que « un long trait » et « trois courts » deviennent le même
    dessin — exactement ce que fait `grid::parse_segment` côté Rust. Un pas qui
    ne serait ni horizontal, ni vertical, ni diagonal à 45° est une erreur du
    catalogue, pas une figure : on s'arrête.
    """
    aretes = set()
    for (r1, c1), (r2, c2) in zip(points, points[1:]):
        dr, dc = r2 - r1, c2 - c1
        if dr == 0 and dc == 0:
            continue
        if dr and dc and abs(dr) != abs(dc):
            raise ValueError(f"pente impossible : ({r1},{c1}) → ({r2},{c2})")
        pas = max(abs(dr), abs(dc))
        sr, sc = dr // pas, dc // pas
        for i in range(pas):
            aretes.add(norm_arete((r1 + i * sr, c1 + i * sc),
                                  (r1 + (i + 1) * sr, c1 + (i + 1) * sc)))
    return aretes


def payload_cases(taille, cases):
    corps = ";".join(f"{r},{c}" for r, c in sorted(cases))
    return f"{taille}x{taille}:c={corps}"


def payload_lignes(taille, aretes):
    corps = ";".join(f"{a[0]},{a[1]}-{b[0]},{b[1]}" for a, b in sorted(aretes))
    return f"{taille}x{taille}:e={corps}"


# ===== Les huit orientations du plan ========================================
#
# Quatre rotations d'un quart de tour, chacune avec et sans miroir. Une forme
# symétrique retombe sur elle-même — la déduplication le voit et ne garde qu'un
# exemplaire, si bien qu'une croix ne donne qu'un dessin et un L en donne huit.
# Aucun risque de servir deux fois la même chose à l'enfant.
#
# Attention à la différence de repère entre les deux types :
#   * une CASE vit dans 0..bh-1 × 0..bw-1  → le miroir renvoie sur bw-1-c ;
#   * un SOMMET vit dans 0..bh × 0..bw     → le miroir renvoie sur bw-c.
# Confondre les deux décalerait la figure d'une demi-case à chaque quart de tour.


def normalise_cases(cases):
    r0 = min(r for r, _ in cases)
    c0 = min(c for _, c in cases)
    dec = frozenset((r - r0, c - c0) for r, c in cases)
    return dec, (max(r for r, _ in dec) + 1, max(c for _, c in dec) + 1)


def normalise_aretes(aretes):
    pts = [p for a in aretes for p in a]
    r0, c0 = min(p[0] for p in pts), min(p[1] for p in pts)
    dec = frozenset(norm_arete((a[0] - r0, a[1] - c0), (b[0] - r0, b[1] - c0))
                    for a, b in aretes)
    pts = [p for a in dec for p in a]
    return dec, (max(p[0] for p in pts), max(p[1] for p in pts))


def orientations_cases(cases):
    vues, vus, cur = [], set(), cases
    for _ in range(4):
        cur, (bh, bw) = normalise_cases(cur)
        for fig in (cur, frozenset((r, bw - 1 - c) for r, c in cur)):
            n, box = normalise_cases(fig)
            if n not in vus:
                vus.add(n)
                vues.append((n, box))
        cur = frozenset((c, bh - 1 - r) for r, c in cur)  # quart de tour
    return vues


def orientations_aretes(aretes):
    vues, vus, cur = [], set(), aretes
    for _ in range(4):
        cur, (bh, bw) = normalise_aretes(cur)
        miroir = frozenset(norm_arete((a[0], bw - a[1]), (b[0], bw - b[1]))
                           for a, b in cur)
        for fig in (cur, miroir):
            n, box = normalise_aretes(fig)
            if n not in vus:
                vus.add(n)
                vues.append((n, box))
        cur = frozenset(norm_arete((a[1], bh - a[0]), (b[1], bh - b[0]))
                        for a, b in cur)
    return vues


# ===== Difficulté ===========================================================


def difficulte(taille, marques):
    """La taille pose le socle, le nombre de marques l'ajuste.

    Une 4×4 reste facile même bien remplie ; une 8×8 chargée est le sommet de
    l'exercice. La plage par enfant (`children.difficulty_min/max`) fait ensuite
    le tri : on peut donner les 4×4 à un enfant qui débute sans jamais lui
    servir de 8×8.

    Les seuils sont ABSOLUS et non un multiple du côté : sur une 8×8 le seuil
    proportionnel (3 × 8 = 24 marques) tombait au-delà de ce que le catalogue
    produit réellement, et le niveau 5 se retrouvait avec UNE figure — un enfant
    réglé là n'aurait jamais eu que le même dessin. Ils sont calés sur la
    distribution mesurée : médiane autour de 8, extrêmes vers 30.
    """
    base = {4: 1, 6: 2, 8: 3}[taille]
    if marques >= 8:
        base += 1
    if marques >= 16:
        base += 1
    return min(5, base)


def deux_calibres(candidats):
    """Ordonne les boîtes « une petite d'abord, puis une grande ».

    Tirer deux tailles au hasard laissait la difficulté s'agglutiner au milieu.
    On propose donc d'abord le premier tiers (les petites), puis le dernier (les
    grandes), chacun mélangé — l'appelant retient la première boîte de chaque
    groupe qui donne une figure exploitable.

    Le milieu ferme la marche de chaque groupe : certaines formes se rabotent
    elles-mêmes pour rester symétriques (losange, maison, sapin) et ne produisent
    rien dans une boîte donnée. Sans ce filet, chaque échec coûtait une figure.
    """
    tri = sorted(candidats, key=lambda b: (b[0] * b[1], b))
    tiers = max(1, len(tri) // 3)
    petites, grandes, milieu = list(tri[:tiers]), list(tri[-tiers:]), list(tri[tiers:-tiers])
    for groupe in (petites, grandes, milieu):
        RNG.shuffle(groupe)
    return [petites + milieu, grandes + milieu]


# ===== Fabrication ==========================================================

questions = []
deja_vues = set()   # figures déjà produites, TOUTES formes et tailles confondues


def ajoute(kind, taille, payload, marques):
    n = len(questions) + 1
    questions.append({
        "subject": SUBJECT,
        "kind": kind,
        # Volontairement court, et surtout SANS répéter la consigne : la page du
        # contrôle affiche déjà « Colorie les mêmes cases que sur le modèle » ou
        # « Trace les mêmes traits », selon le type. Le dire deux fois, c'est du
        # texte de plus à déchiffrer pour un enfant qui vient travailler le
        # repérage, pas la lecture.
        #
        # Le numéro, lui, n'est pas décoratif : dans la liste du panel admin,
        # des centaines d'énoncés identiques seraient impossibles à parcourir
        # pour le parent qui cherche une figure précise.
        "statement": f"Dessin n° {n} — regarde bien le modèle, puis refais-le sur la grille vide.",
        "answers": [{"text": payload, "correct": True}],
        "explanation": RNG.choice(CONSEILS),
        "difficulty": difficulte(taille, marques),
    })


def pose(kind, taille, figure, box, fabrique_payload):
    """Place une figure au hasard dans la grille, si elle y tient.

    Rend True quand un dessin a bien été produit — c'est-à-dire quand la figure
    tient dans la grille ET qu'elle n'a jamais été vue ailleurs. Ce second point
    compte : avec les huit orientations, deux formes différentes finissent
    parfois sur le même dessin (un L retourné est un J), et l'enfant n'a rien à
    gagner à le recopier deux fois.
    """
    bh, bw = box
    if bh > taille or bw > taille:
        return False
    r0 = RNG.randint(0, taille - bh)
    c0 = RNG.randint(0, taille - bw)
    if kind == "grid_cells":
        place = {(r + r0, c + c0) for r, c in figure}
    else:
        place = {norm_arete((a[0] + r0, a[1] + c0), (b[0] + r0, b[1] + c0))
                 for a, b in figure}
    payload = fabrique_payload(taille, place)
    if payload in deja_vues:
        return False
    deja_vues.add(payload)
    ajoute(kind, taille, payload, len(place))
    return True


def genere(kind, formes, base_figure, orientations, fabrique_payload, marques_min):
    recolte = Counter()
    for taille in SIZES:
        for nom, fn, hmin, wmin in formes:
            candidats = [(bh, bw)
                         for bh in range(hmin, taille + 1)
                         for bw in range(wmin, taille + 1)]
            if not candidats:
                continue
            for groupe in deux_calibres(candidats):
                for bh, bw in groupe:
                    fig = base_figure(fn, bh, bw)
                    if fig is None or len(fig) < marques_min:
                        continue
                    # Les orientations sont mélangées : sans ça on servirait
                    # toujours la forme droite en premier, et les variantes
                    # tournées — celles qui font justement travailler le
                    # repérage — ne sortiraient que sur les formes prolifiques.
                    variantes = orientations(fig)
                    RNG.shuffle(variantes)
                    posees = 0
                    for figure, box in variantes:
                        if posees >= ORIENTATIONS:
                            break
                        if pose(kind, taille, figure, box, fabrique_payload):
                            posees += 1
                    if posees:
                        recolte[nom] += posees
                        break   # ce calibre a donné : on passe au suivant

    # Une forme qui n'a RIEN produit sur aucune taille n'est pas une forme
    # difficile à caser : c'est une forme cassée — une pente impossible à
    # tracer, ou une boîte minimale mal déclarée. Sans ce garde-fou elle
    # disparaîtrait en silence, et on croirait le catalogue plus riche qu'il
    # n'est. (À l'inverse, échouer sur CERTAINES boîtes est normal : le drapeau
    # n'a sa diagonale à 45° que lorsque la hauteur vaut le double de la
    # largeur.)
    muettes = [nom for nom, _, _, _ in formes if not recolte[nom]]
    if muettes:
        print(f"formes muettes ({kind}) : {', '.join(muettes)}", file=sys.stderr)
        sys.exit(1)


def base_cases(fn, bh, bw):
    cases = fn(bh, bw)
    return cases or None


def base_lignes(fn, bh, bw):
    """Rend None quand la forme n'est pas traçable dans CETTE boîte.

    Beaucoup de figures ne tiennent leurs 45° que dans une boîte carrée, ou de
    proportions données. Refuser tout le script au premier échec reviendrait à
    n'accepter que des formes universelles ; c'est `genere` qui vérifie, à la
    fin, qu'aucune forme n'est restée muette PARTOUT.
    """
    try:
        aretes = set()
        for chemin in fn(bh, bw):
            aretes |= chemin_en_aretes(chemin)
    except ValueError:
        return None
    return aretes or None


genere("grid_cells", FORMES_CASES, base_cases, orientations_cases, payload_cases, 3)
genere("grid_lines", FORMES_LIGNES, base_lignes, orientations_aretes, payload_lignes, 3)

# ===== Vérification — les mêmes règles que src/importer.rs et src/grid.rs ====

erreurs = []
figures = set()
for i, q in enumerate(questions):
    ou = f"question #{i + 1}"
    if not q["statement"].strip():
        erreurs.append(f"{ou} : énoncé vide")
    if len(q["answers"]) != 1 or not q["answers"][0]["correct"]:
        erreurs.append(f"{ou} : il faut exactement 1 réponse, marquée correcte")
        continue
    if not 1 <= q["difficulty"] <= 5:
        erreurs.append(f"{ou} : difficulté {q['difficulty']} hors de [1,5]")

    payload = q["answers"][0]["text"]
    if payload in figures:
        erreurs.append(f"{ou} : figure en double — {payload}")
    figures.add(payload)

    dims, reste = payload.split(":", 1)
    w, h = (int(x) for x in dims.split("x"))
    marqueur, corps = reste.split("=", 1)
    jetons = [t for t in corps.split(";") if t]
    if not jetons:
        erreurs.append(f"{ou} : figure vide, rien à reproduire")
    attendu = "c" if q["kind"] == "grid_cells" else "e"
    if marqueur != attendu:
        erreurs.append(f"{ou} : marqueur '{marqueur}' pour un type {q['kind']}")

    if marqueur == "c":
        for t in jetons:
            r, c = (int(x) for x in t.split(","))
            if not (0 <= r < h and 0 <= c < w):
                erreurs.append(f"{ou} : case {t} hors de la grille {w}×{h}")
    else:
        for t in jetons:
            a, b = t.split("-")
            r1, c1 = (int(x) for x in a.split(","))
            r2, c2 = (int(x) for x in b.split(","))
            for r, c in ((r1, c1), (r2, c2)):
                # Le treillis a un point de plus que de cases, dans les deux sens.
                if not (0 <= r <= h and 0 <= c <= w):
                    erreurs.append(f"{ou} : point {r},{c} hors du treillis {w}×{h}")
            dr, dc = r2 - r1, c2 - c1
            if (dr, dc) == (0, 0) or max(abs(dr), abs(dc)) != 1:
                erreurs.append(f"{ou} : {t} n'est pas une arête d'une seule case")
    # Forme canonique : le fichier doit déjà être trié, sinon il diffèrera de ce
    # que le serveur enregistre et les diffs Git deviendront illisibles.
    if jetons != sorted(jetons, key=lambda t: [
            [int(n) for n in p.split(",")] for p in t.split("-")]):
        erreurs.append(f"{ou} : jetons non triés")

if erreurs:
    print("\n".join(erreurs[:40]), file=sys.stderr)
    if len(erreurs) > 40:
        print(f"… et {len(erreurs) - 40} autres", file=sys.stderr)
    sys.exit(1)

with open(OUT, "w", encoding="utf-8") as f:
    json.dump({"subjects": [{"name": SUBJECT, "weight": 1.0}], "questions": questions},
              f, ensure_ascii=False, indent=2)
    f.write("\n")

par_taille_diff = Counter()
for q in questions:
    t = int(q["answers"][0]["text"].split("x")[0])
    par_taille_diff[(t, q["difficulty"])] += 1

print(f"OK — {len(questions)} figures écrites dans {OUT}")
print("  types      :", dict(Counter(q["kind"] for q in questions)))
print("  difficultés:", dict(sorted(Counter(q["difficulty"] for q in questions).items())))
print("\n  taille × difficulté")
for t in SIZES:
    print(f"   {t}x{t}   " + "  ".join(
        f"d{d}:{par_taille_diff[(t, d)]:3d}" for d in range(1, 6)))
