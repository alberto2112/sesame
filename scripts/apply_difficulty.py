#!/usr/bin/env python3
"""Écrit la difficulté proposée dans data/questions_<matière>.json.

La banque d'origine ne déclare AUCUNE difficulté. L'importeur leur applique donc
son défaut — 3 (importer.rs::default_difficulty) — et les 390 questions y
atterrissent toutes. Ce n'est pas une note, c'est une absence de note qui en a
l'air : un enfant dont la plage exclut 3 ne voit alors NI les 45 questions à
réponses multiples, NI la moindre question de Culture générale.

Les notes viennent d'un fichier de propositions par matière :

    [ {"statement": "<énoncé exact>", "difficulty": 2}, ... ]

L'appariement se fait sur l'ÉNONCÉ, pas sur la position : une proposition dans
le désordre reste correcte, une proposition qui invente un énoncé est rejetée.
Le script est TOUT-OU-RIEN par fichier — mieux vaut ne rien écrire qu'une banque
à moitié notée, où l'on ne saurait plus ce qui a été jugé et ce qui est resté au
défaut.

Usage :
    python3 scripts/apply_difficulty.py <dossier-des-propositions> [--dry-run]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data"

# fichier de données  ->  fichier de propositions
PAIRS = {
    "questions_conjugaison.json": "diff_conjugaison.json",
    "questions_culture.json": "diff_culture.json",
    "questions_lecture.json": "diff_lecture.json",
    "questions_mathematiques.json": "diff_mathematiques.json",
    "questions_sciences.json": "diff_sciences.json",
}


def norm(text: str) -> str:
    """Clé d'appariement : mêmes règles que dedup.rs::dedup_key."""
    return re.sub(r"\s+", " ", text.strip()).lower()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("proposals", type=Path, help="dossier contenant les diff_*.json")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    total_ok = 0
    failed = False

    for data_name, prop_name in PAIRS.items():
        data_path = DATA / data_name
        prop_path = args.proposals / prop_name
        print(f"\n{data_name}")

        if not prop_path.is_file():
            print(f"  ✗ proposition manquante : {prop_path}")
            failed = True
            continue

        data = json.loads(data_path.read_text(encoding="utf-8"))
        proposals = json.loads(prop_path.read_text(encoding="utf-8"))
        questions = data["questions"]

        by_key: dict[str, int] = {}
        for p in proposals:
            d = int(p["difficulty"])
            if not 1 <= d <= 5:
                print(f"  ✗ difficulté hors de [1,5] : {d} pour « {p['statement'][:50]} »")
                failed = True
            by_key[norm(p["statement"])] = d

        # Tout-ou-rien : on vérifie AVANT d'écrire quoi que ce soit.
        missing = [q["statement"] for q in questions if norm(q["statement"]) not in by_key]
        if missing:
            print(f"  ✗ {len(missing)} question(s) sans proposition — rien n'est écrit :")
            for m in missing[:5]:
                print(f"      « {m[:60]} »")
            failed = True
            continue

        unknown = len(by_key) - len({norm(q["statement"]) for q in questions})
        if unknown > 0:
            print(f"  ! {unknown} proposition(s) ne correspondent à aucune question — ignorée(s)")

        for q in questions:
            q["difficulty"] = by_key[norm(q["statement"])]

        spread = Counter(q["difficulty"] for q in questions)
        detail = "  ".join(f"d{d}={spread[d]}" for d in range(1, 6) if spread[d])
        flat = len([d for d in range(1, 6) if spread[d]]) == 1
        print(f"  ✓ {len(questions)} notée(s) : {detail}")
        if flat:
            print("  ! toutes à la même note — c'est le problème qu'on corrige, pas une correction")

        if not args.dry_run:
            data_path.write_text(
                json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
            )
        total_ok += len(questions)

    if failed:
        print("\n✗ échec — aucun fichier incomplet n'a été écrit.", file=sys.stderr)
        return 1

    print(f"\n{total_ok} question(s) notée(s)" + ("  [DRY RUN — rien écrit]" if args.dry_run else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
