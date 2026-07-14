#!/usr/bin/env python3
"""Convertit les banques de questions externes vers le format d'import de sesame.

Source (array plat) :
    { "question", "type": "choice"|"exact", "correct", "difficulty",
      "explanation", "options"? }

Cible (src/importer.rs — ImportFile) :
    { "subjects": [{ "name", "weight" }],
      "questions": [{ "subject", "kind", "statement", "answers": [{"text","correct"}],
                      "explanation", "difficulty" }] }

Correspondance des types :
    choice              -> single   (les options ; `correct` est le TEXTE de la bonne)
    exact, réponse numérique -> number   (l'enfant écrit ; comparaison numérique)
    exact, réponse texte     -> exact    (l'enfant écrit ; casse et espaces ignorés)

Aucun distracteur n'est inventé pour les réponses libres : calculer « 47 + 38 »
n'est pas la même chose que le reconnaître parmi quatre options — le second se
devine par élimination. Et la bonne réponse est STOCKÉE, jamais recalculée depuis
l'énoncé : un énoncé n'est pas toujours une expression (« 0, 1, 2, 3, ... ? »).

Ce que le script ne sait pas convertir finit dans pending/rejects.json, avec sa
raison — jamais silencieusement écarté.

Usage :
    python3 scripts/convert_questions.py [--src /tmp/import] [--dry-run]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

# --- Configuration : fichier source -> matière (mapping « hybride ») ---------
# Les matières déjà présentes dans data/ sont réutilisées ; « Logique » est
# nouvelle : c'est du raisonnement, pas de l'arithmétique, et on veut pouvoir
# la pondérer à part depuis /admin.
FILE_SUBJECT: dict[str, str] = {
    "animaux": "Sciences",
    "nature": "Sciences",
    "comprehension": "Lecture et vocabulaire",
    "grammaire": "Lecture et vocabulaire",
    "orthographe": "Lecture et vocabulaire",
    "conjugaisons": "Conjugaison",
    "additions": "Mathématiques",
    "soustractions": "Mathématiques",
    "logique": "Logique",
}

# Nom court utilisé dans le nom du fichier de sortie (import_<slug>.json).
SUBJECT_SLUG: dict[str, str] = {
    "Sciences": "sciences",
    "Lecture et vocabulaire": "lecture",
    "Conjugaison": "conjugaison",
    "Mathématiques": "mathematiques",
    "Logique": "logique",
}

DEFAULT_WEIGHT = 1.0

REPO = Path(__file__).resolve().parent.parent
OUT_DIR = REPO / "data"
PENDING_DIR = REPO / "pending"


def norm(text: str) -> str:
    """Clé de comparaison : sert uniquement à repérer les doublons."""
    return re.sub(r"\s+", " ", text.strip().lower())


def validate(q: dict) -> str | None:
    """Rejoue les règles de importer.rs::validate_question. None = valide.

    Rejouées ici pour que le script échoue à l'écriture, pas 3000 questions plus
    tard à l'import. Les deux doivent rester d'accord : si tu changes l'une,
    change l'autre.
    """
    if not q["statement"].strip():
        return "énoncé vide"
    if not 1 <= q["difficulty"] <= 5:
        return f"difficulté {q['difficulty']} hors de [1,5]"
    for i, a in enumerate(q["answers"], start=1):
        if not a["text"].strip():
            return f"texte de la réponse #{i} vide"
    correct = sum(1 for a in q["answers"] if a["correct"])

    if q["kind"] in ("exact", "number"):
        if len(q["answers"]) != 1 or correct != 1:
            return f"type '{q['kind']}' exige exactement 1 réponse, marquée correcte"
        if q["kind"] == "number" and not NUMERIC.match(q["answers"][0]["text"]):
            return f"type 'number' : '{q['answers'][0]['text']}' n'est pas un nombre"
        return None

    if len(q["answers"]) < 2:
        return f"au moins 2 réponses requises, {len(q['answers'])} fournies"
    if q["kind"] == "single" and correct != 1:
        return f"type 'single' exige exactement 1 réponse correcte, {correct} trouvées"
    if q["kind"] == "multi" and (correct < 1 or correct == len(q["answers"])):
        return "type 'multi' exige au moins 1 réponse correcte et 1 incorrecte"
    return None


def convert_choice(item: dict, subject: str) -> dict:
    """choice -> single. `correct` est le TEXTE de la bonne option."""
    options = item["options"]
    target = item["correct"]
    return {
        "subject": subject,
        "kind": "single",
        "statement": item["question"].strip(),
        "answers": [
            {"text": str(o).strip(), "correct": str(o) == str(target)} for o in options
        ],
        "explanation": (item.get("explanation") or "").strip() or None,
        "difficulty": int(item["difficulty"]),
    }


NUMERIC = re.compile(r"^[+-]?\d+([.,]\d+)?$")


def convert_exact(item: dict, subject: str) -> dict:
    """exact (réponse libre) -> 'number' si la réponse est un nombre, sinon 'exact'.

    Les deux stockent LA bonne réponse (une seule ligne, correcte) — elle n'est
    jamais recalculée depuis l'énoncé. Seule la COMPARAISON diffère :
      number → numérique  (« 08 » == « 8 » == « 8,0 » == « +8 »)
      exact  → texte, rogné et insensible à la casse (les accents comptent)
    """
    correct = str(item["correct"]).strip()
    return {
        "subject": subject,
        "kind": "number" if NUMERIC.match(correct) else "exact",
        "statement": item["question"].strip(),
        "answers": [{"text": correct, "correct": True}],
        "explanation": (item.get("explanation") or "").strip() or None,
        "difficulty": int(item["difficulty"]),
    }


def existing_statements() -> set[str]:
    """Énoncés déjà présents dans data/questions_*.json, pour signaler les collisions."""
    seen: set[str] = set()
    for path in OUT_DIR.glob("questions_*.json"):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        for q in data.get("questions", []):
            if "statement" in q:
                seen.add(norm(q["statement"]))
    return seen


def envelope(subject: str, questions: list[dict]) -> dict:
    return {
        "subjects": [{"name": subject, "weight": DEFAULT_WEIGHT}],
        "questions": questions,
    }


def write_json(path: Path, payload: dict, dry_run: bool) -> None:
    if dry_run:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--src", type=Path, default=Path("/tmp/import"))
    ap.add_argument("--dry-run", action="store_true", help="n'écrit rien, rapport seul")
    args = ap.parse_args()

    if not args.src.is_dir():
        print(f"erreur : {args.src} introuvable", file=sys.stderr)
        return 1

    importable: dict[str, list[dict]] = defaultdict(list)  # subject -> questions
    rejected: list[dict] = []

    known = existing_statements()
    seen: dict[str, str] = {}  # énoncé normalisé -> origine
    collisions: list[tuple[str, str]] = []
    per_file: list[tuple[str, str, Counter]] = []  # fichier, matière, kinds

    for path in sorted(args.src.glob("*.json")):
        subject = FILE_SUBJECT.get(path.stem)
        if subject is None:
            print(f"  ! {path.name} : aucune matière mappée — ignoré", file=sys.stderr)
            continue

        items = json.loads(path.read_text(encoding="utf-8"))
        kinds: Counter = Counter()

        for idx, item in enumerate(items):
            src_type = item.get("type")
            try:
                if src_type == "choice":
                    q = convert_choice(item, subject)
                elif src_type == "exact":
                    q = convert_exact(item, subject)
                else:
                    rejected.append(
                        {"source": path.name, "index": idx,
                         "reason": f"type source '{src_type}' inconnu", "item": item}
                    )
                    continue
            except (KeyError, TypeError, ValueError) as e:
                rejected.append(
                    {"source": path.name, "index": idx,
                     "reason": f"item malformé : {e}", "item": item}
                )
                continue

            key = norm(q["statement"])
            if key in known:
                collisions.append((path.name, q["statement"]))
            if key in seen:
                rejected.append(
                    {"source": path.name, "index": idx,
                     "reason": f"doublon de {seen[key]}", "item": item}
                )
                continue
            seen[key] = path.name

            err = validate(q)
            if err:
                rejected.append(
                    {"source": path.name, "index": idx, "reason": err, "item": item}
                )
                continue
            importable[subject].append(q)
            kinds[q["kind"]] += 1

        per_file.append((path.name, subject, kinds))

    # --- écriture ----------------------------------------------------------
    for subject, questions in sorted(importable.items()):
        slug = SUBJECT_SLUG.get(subject, norm(subject).replace(" ", "_"))
        write_json(OUT_DIR / f"import_{slug}.json", envelope(subject, questions), args.dry_run)

    if rejected:
        write_json(PENDING_DIR / "rejects.json", {"rejected": rejected}, args.dry_run)

    # --- rapport -----------------------------------------------------------
    print(f"\nSource : {args.src}" + ("  [DRY RUN — rien écrit]" if args.dry_run else ""))
    print("\n  fichier                matière                  single  number   exact")
    print("  " + "-" * 68)
    for name, subject, kinds in per_file:
        print(
            f"  {name:<22} {subject:<24}"
            f"{kinds['single']:>6}  {kinds['number']:>6}  {kinds['exact']:>6}"
        )

    total_ok = sum(len(v) for v in importable.values())
    print(f"\nImportables ({total_ok}) → data/")
    for subject, questions in sorted(importable.items()):
        slug = SUBJECT_SLUG.get(subject, norm(subject).replace(" ", "_"))
        by_kind = Counter(q["kind"] for q in questions)
        detail = ", ".join(f"{n} {k}" for k, n in sorted(by_kind.items()))
        print(f"  import_{slug}.json  —  {subject}  ({len(questions)} : {detail})")

    print(f"\nRejetées ({len(rejected)}) → pending/rejects.json")
    if collisions:
        print(f"\n! {len(collisions)} énoncé(s) déjà présent(s) dans data/questions_*.json :")
        for name, statement in collisions[:10]:
            print(f"    [{name}] {statement[:60]}")
        if len(collisions) > 10:
            print(f"    … et {len(collisions) - 10} de plus")

    if not args.dry_run and total_ok:
        print("\nPour importer :")
        for subject in sorted(importable):
            slug = SUBJECT_SLUG.get(subject, norm(subject).replace(" ", "_"))
            print(f"  cargo run --bin sesame -- import data/import_{slug}.json")
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
