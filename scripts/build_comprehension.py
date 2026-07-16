#!/usr/bin/env python3
"""Fusionne les 6 lots de compréhension, valide, et écrit data/questions_comprehension.json.

Valide contre les règles de src/importer.rs + des règles de QUALITÉ propres aux QCM :
  - la bonne réponse ne doit pas être systématiquement la plus longue (sinon on devine sans lire)
  - pas de collision d'énoncé avec les banques existantes
  - calibrage de longueur du texte par niveau
"""
import glob
import json
import sys
from collections import Counter

SCRATCH = "/private/tmp/claude-501/-Users-alberto-Workspace-Rust-luanti/4d19e201-54d5-4676-8f56-db183578dd3a/scratchpad"
SUBJECT = "Compréhension"
OUT = "data/questions_comprehension.json"

# L'ÉCHELLE difficulty EST L'ANNÉE SCOLAIRE. C'est le seul filtre par enfant
# qu'expose pick_questions (children.difficulty_min/max, migration 0004), donc
# elle doit vouloir dire LA MÊME CHOSE dans tous les subjects :
#   1 = CP / début CE1   2 = CE1   3 = CE2   4 = CM1   5 = CM2
# `remap` réattribue la difficulté produite par les rédacteurs vers cette échelle.
LOTS = [
    # fichier,             niveau, difficultés acceptées en entrée, remap
    ("comp_ce1_a.json", "CE1", {1, 2}, None),      # d1 = début CE1, d2 = CE1 : on garde
    ("comp_ce1_b.json", "CE1", {1, 2}, None),
    ("comp_ce2_c.json", "CE2", {3}, None),         # déjà sur l'échelle
    ("comp_ce2_d.json", "CE2", {3}, None),
    ("comp_cm1_g.json", "CM1", {4}, None),         # déjà sur l'échelle
    ("comp_cm1_h.json", "CM1", {4}, None),
    ("comp_cm2_e.json", "CM2", {4, 5}, 5),         # tout le CM2 → d5 (d4 est pris par le CM1)
    ("comp_cm2_f.json", "CM2", {4, 5}, 5),
]

# longueur de statement (texte + question) attendue par niveau — bornes larges
LEN_RANGE = {"CE1": (60, 320), "CE2": (140, 480), "CM1": (200, 650), "CM2": (280, 900)}

errors, warnings = [], []
allq = []

for fname, level, diffs, remap in LOTS:
    path = f"{SCRATCH}/{fname}"
    try:
        with open(path, encoding="utf-8") as f:
            lot = json.load(f)
    except FileNotFoundError:
        errors.append(f"{fname} : FICHIER MANQUANT")
        continue
    except json.JSONDecodeError as e:
        errors.append(f"{fname} : JSON INVALIDE — {e}")
        continue
    if not isinstance(lot, list):
        errors.append(f"{fname} : attendu un tableau JSON, reçu {type(lot).__name__}")
        continue
    if len(lot) != 50:
        warnings.append(f"{fname} : {len(lot)} questions au lieu de 50")
    for q in lot:
        q["_lot"], q["_level"] = fname, level
        q["_expected_diffs"] = diffs
        # on valide la difficulté PRODUITE, puis on la ramène sur l'échelle-niveau
        q["_raw_difficulty"] = q.get("difficulty")
        if remap is not None and isinstance(q.get("difficulty"), int):
            q["difficulty"] = remap
        allq.append(q)

# ------------------------------------------------------------------ validation
seen_stmt = {}
for q in allq:
    tag = f"{q['_lot']} « {str(q.get('statement', ''))[:50]}… »"
    stmt = q.get("statement", "")
    kind = q.get("kind", "")
    answers = q.get("answers", [])
    diff = q.get("difficulty")

    # --- règles importer.rs
    if kind not in ("single", "multi"):
        errors.append(f"{tag} : kind '{kind}' interdit ici (single/multi attendus)")
        continue
    raw = q["_raw_difficulty"]
    if not isinstance(raw, int) or not (1 <= raw <= 5):
        errors.append(f"{tag} : difficulty invalide ({raw!r})")
    elif raw not in q["_expected_diffs"]:
        errors.append(f"{tag} : difficulty {raw} hors du niveau {q['_level']} {sorted(q['_expected_diffs'])}")
    if not isinstance(diff, int) or not (1 <= diff <= 5):
        errors.append(f"{tag} : difficulty remappée invalide ({diff!r})")
    if len(answers) < 2:
        errors.append(f"{tag} : moins de 2 réponses")
        continue
    correct = sum(1 for a in answers if a.get("correct"))
    texts = [str(a.get("text", "")).strip().lower() for a in answers]
    if any(not t for t in texts):
        errors.append(f"{tag} : réponse vide")
    if len(texts) != len(set(texts)):
        errors.append(f"{tag} : options dupliquées")
    if kind == "single" and correct != 1:
        errors.append(f"{tag} : single avec {correct} bonnes réponses")
    if kind == "multi" and (correct == 0 or correct == len(answers)):
        errors.append(f"{tag} : multi avec {correct}/{len(answers)} bonnes réponses")
    if not str(q.get("explanation", "")).strip():
        errors.append(f"{tag} : explication vide")

    # --- règles de rendu / format
    if "\n" in stmt:
        errors.append(f"{tag} : contient un saut de ligne (le HTML l'avale)")
    if '"' in stmt:
        warnings.append(f"{tag} : contient le caractère \" (préférer « »)")

    # --- unicité
    key = " ".join(stmt.split()).lower()
    if key in seen_stmt:
        errors.append(f"{tag} : ÉNONCÉ DUPLIQUÉ (déjà dans {seen_stmt[key]})")
    seen_stmt[key] = q["_lot"]

    # --- calibrage
    lo, hi = LEN_RANGE[q["_level"]]
    if not (lo <= len(stmt) <= hi):
        warnings.append(f"{tag} : longueur {len(stmt)} hors calibrage {q['_level']} [{lo},{hi}]")

# ------------------------------------- qualité : la bonne réponse trahit-elle ?
# Si la bonne réponse est systématiquement la plus longue des 4, l'enfant la coche
# SANS LIRE LE TEXTE et décroche l'ordinateur sans rien comprendre. Le hasard donne
# 25 %. Au-delà de 40 %, le lot est cassé : on BLOQUE, on n'avertit pas.
# On mesure les DEUX sens. Un lot « corrigé » en rendant la bonne réponse toujours la
# PLUS COURTE serait tout aussi jouable : l'enfant apprend l'autre règle, c'est tout.
SEUIL = 40
by_lot = {}
for q in allq:
    if q.get("kind") != "single" or len(q.get("answers", [])) < 2:
        continue
    good = [a for a in q["answers"] if a.get("correct")]
    bad = [a for a in q["answers"] if not a.get("correct")]
    if not good or not bad:
        continue
    n = len(good[0]["text"])
    longest = n > max(len(a["text"]) for a in bad)
    shortest = n < min(len(a["text"]) for a in bad)
    by_lot.setdefault((q["_level"], q["_lot"]), []).append((longest, shortest))

print(f"— La bonne réponse trahit-elle sa position par sa LONGUEUR ? (hasard = 25 %, seuil = {SEUIL} %)")
print(f"   {'niveau':<6} {'lot':<19} {'+longue':>8} {'+courte':>8}")
for (lvl, lot), v in sorted(by_lot.items()):
    pl = 100 * sum(a for a, _ in v) / len(v)
    pc = 100 * sum(b for _, b in v) / len(v)
    ok = pl <= SEUIL and pc <= SEUIL
    print(f"   {lvl:<6} {lot:<19} {pl:>7.0f} % {pc:>7.0f} %   "
          f"{'OK' if ok else '✗ DEVINABLE SANS LIRE LE TEXTE'}")
    if pl > SEUIL:
        errors.append(f"{lot} ({lvl}) : bonne réponse LA PLUS LONGUE dans {pl:.0f} % des cas (> {SEUIL} %)")
    if pc > SEUIL:
        errors.append(f"{lot} ({lvl}) : bonne réponse LA PLUS COURTE dans {pc:.0f} % des cas (> {SEUIL} %)")

# ------------------------------------------- collision avec les banques existantes
old = set()
for f in glob.glob("data/*.json"):
    if f.endswith("questions_comprehension.json"):
        continue
    try:
        d = json.load(open(f, encoding="utf-8"))
    except Exception:
        continue
    for q in d.get("questions", []):
        old.add(" ".join(str(q.get("statement", "")).split()).lower())
collisions = [s for s in seen_stmt if s in old]
print(f"\n— Collisions avec la banque existante ({len(old)} énoncés) : {len(collisions)}")
for c in collisions:
    errors.append(f"COLLISION avec la banque existante : « {c[:70]}… »")

# ------------------------------------------------------------------- rapport
print(f"\n— Total : {len(allq)} questions")
print("   niveaux    :", dict(Counter(q["_level"] for q in allq)))
print("   difficultés:", dict(sorted(Counter(q.get("difficulty") for q in allq).items())))
print("   kinds      :", dict(Counter(q.get("kind") for q in allq)))

if warnings:
    print(f"\n⚠ {len(warnings)} AVERTISSEMENTS")
    for w in warnings[:25]:
        print("   ", w)
    if len(warnings) > 25:
        print(f"    … et {len(warnings) - 25} autres")

if errors:
    print(f"\n✗ {len(errors)} ERREURS — RIEN N'EST ÉCRIT")
    for e in errors[:40]:
        print("   ", e)
    if len(errors) > 40:
        print(f"    … et {len(errors) - 40} autres")
    sys.exit(1)

# ---------------------------------------------------------------------- écriture
out = {
    "subjects": [{"name": SUBJECT, "weight": 1.0}],
    "questions": [
        {
            "subject": SUBJECT,
            "kind": q["kind"],
            "statement": " ".join(q["statement"].split()),
            "answers": [{"text": a["text"].strip(), "correct": bool(a.get("correct"))} for a in q["answers"]],
            "explanation": " ".join(q["explanation"].split()),
            "difficulty": q["difficulty"],
        }
        for q in allq
    ],
}
with open(OUT, "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
    f.write("\n")
print(f"\n✓ {len(out['questions'])} questions écrites dans {OUT}")
