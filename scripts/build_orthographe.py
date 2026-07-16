#!/usr/bin/env python3
"""Fusionne les 4 lots singulier/pluriel, valide, écrit data/questions_orthographe.json.

Échelle difficulty = ANNÉE SCOLAIRE (1=CP, 2=CE1, 3=CE2, 4=CM1, 5=CM2).
Ici : CM1 → 4, CM2 → 5.

Contrôles SPÉCIFIQUES au kind 'exact' (src/quiz.rs : trim + lowercase, rien d'autre) :
  - la réponse doit être UN SEUL MOT (un espace = l'enfant est recalé sur un article)
  - la réponse ne doit pas commencer par un article
  - l'énoncé doit dire explicitement ce qu'on attend
"""
import glob
import json
import re
import sys
from collections import Counter

SCRATCH = "/private/tmp/claude-501/-Users-alberto-Workspace-Rust-luanti/4d19e201-54d5-4676-8f56-db183578dd3a/scratchpad"
SUBJECT = "Orthographe"
OUT = "data/questions_orthographe.json"

LOTS = [
    ("plur_cm1_a.json", "CM1", 4, 38),
    ("plur_cm1_b.json", "CM1", 4, 37),
    ("plur_cm2_c.json", "CM2", 5, 38),
    ("plur_cm2_d.json", "CM2", 5, 37),
]

ARTICLES = {"le", "la", "les", "un", "une", "des", "l", "du", "de", "au", "aux"}

errors, warnings = [], []
allq = []

for fname, level, diff, expected_n in LOTS:
    try:
        with open(f"{SCRATCH}/{fname}", encoding="utf-8") as f:
            lot = json.load(f)
    except FileNotFoundError:
        errors.append(f"{fname} : FICHIER MANQUANT")
        continue
    except json.JSONDecodeError as e:
        errors.append(f"{fname} : JSON INVALIDE — {e}")
        continue
    if not isinstance(lot, list):
        errors.append(f"{fname} : attendu un tableau JSON")
        continue
    if len(lot) != expected_n:
        warnings.append(f"{fname} : {len(lot)} questions au lieu de {expected_n}")
    for q in lot:
        q["_lot"], q["_level"], q["_diff"] = fname, level, diff
        allq.append(q)

seen = {}
for q in allq:
    tag = f"{q['_lot']} « {str(q.get('statement',''))[:48]}… »"
    stmt = str(q.get("statement", ""))
    kind = q.get("kind", "")
    answers = q.get("answers", [])

    if kind not in ("single", "multi", "exact"):
        errors.append(f"{tag} : kind '{kind}' interdit (single/multi/exact)")
        continue
    if not answers:
        errors.append(f"{tag} : aucune réponse")
        continue
    if "\n" in stmt:
        errors.append(f"{tag} : contient un saut de ligne")
    if '"' in stmt:
        warnings.append(f"{tag} : contient le caractère \" (préférer « »)")
    if not str(q.get("explanation", "")).strip():
        errors.append(f"{tag} : explication vide")

    correct = sum(1 for a in answers if a.get("correct"))
    texts = [str(a.get("text", "")).strip().lower() for a in answers]
    if any(not t for t in texts):
        errors.append(f"{tag} : réponse vide")
    if len(texts) != len(set(texts)):
        errors.append(f"{tag} : options dupliquées")

    if kind == "exact":
        # règles importer : exactement 1 réponse, correcte
        if len(answers) != 1 or not answers[0].get("correct"):
            errors.append(f"{tag} : 'exact' exige EXACTEMENT 1 réponse correcte ({len(answers)} fournies)")
            continue
        ans = str(answers[0]["text"]).strip()
        # la correction est trim+lowercase : tout écart de forme = enfant recalé à tort
        if " " in ans:
            errors.append(f"{tag} : réponse 'exact' en PLUSIEURS MOTS (« {ans} ») — l'enfant sera recalé sur l'article/l'espace")
        if ans.split("'")[0].lower() in ARTICLES or ans.split(" ")[0].lower() in ARTICLES:
            errors.append(f"{tag} : réponse 'exact' commençant par un article (« {ans} »)")
        if not re.search(r"uniquement|seulement|sans article", stmt, re.IGNORECASE):
            errors.append(f"{tag} : énoncé 'exact' SANS consigne explicite (« écris UNIQUEMENT … ») — piège de format")
    elif kind == "single":
        if len(answers) < 2:
            errors.append(f"{tag} : single avec moins de 2 options")
        if correct != 1:
            errors.append(f"{tag} : single avec {correct} bonnes réponses")
    elif kind == "multi":
        if correct == 0 or correct == len(answers):
            errors.append(f"{tag} : multi avec {correct}/{len(answers)} bonnes réponses")

    key = " ".join(stmt.split()).lower()
    if key in seen:
        errors.append(f"{tag} : ÉNONCÉ DUPLIQUÉ (déjà dans {seen[key]})")
    seen[key] = q["_lot"]

# --- collisions avec les banques existantes
old = set()
for f in glob.glob("data/*.json"):
    if f.endswith("questions_orthographe.json"):
        continue
    try:
        d = json.load(open(f, encoding="utf-8"))
    except Exception:
        continue
    for q in d.get("questions", []):
        old.add(" ".join(str(q.get("statement", "")).split()).lower())
for s in seen:
    if s in old:
        errors.append(f"COLLISION avec la banque existante : « {s[:70]}… »")

print(f"— Total : {len(allq)} questions")
print("   niveaux:", dict(Counter(q["_level"] for q in allq)))
print("   kinds  :", dict(Counter(q.get("kind") for q in allq)))
print(f"   collisions avec la banque existante ({len(old)} énoncés) : "
      f"{sum(1 for s in seen if s in old)}")

if warnings:
    print(f"\n⚠ {len(warnings)} avertissements")
    for w in warnings[:20]:
        print("   ", w)

if errors:
    print(f"\n✗ {len(errors)} ERREURS — RIEN N'EST ÉCRIT")
    for e in errors[:40]:
        print("   ", e)
    if len(errors) > 40:
        print(f"    … et {len(errors)-40} autres")
    sys.exit(1)

out = {
    "subjects": [{"name": SUBJECT, "weight": 1.0}],
    "questions": [
        {
            "subject": SUBJECT,
            "kind": q["kind"],
            "statement": " ".join(q["statement"].split()),
            "answers": [{"text": str(a["text"]).strip(), "correct": bool(a.get("correct"))} for a in q["answers"]],
            "explanation": " ".join(str(q["explanation"]).split()),
            "difficulty": q["_diff"],
        }
        for q in allq
    ],
}
with open(OUT, "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
    f.write("\n")
print(f"\n✓ {len(out['questions'])} questions écrites dans {OUT}")
print("   difficultés:", dict(sorted(Counter(q["difficulty"] for q in out["questions"]).items())))
