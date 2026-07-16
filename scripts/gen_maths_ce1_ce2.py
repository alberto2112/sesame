#!/usr/bin/env python3
"""Génère 75 questions de mathématiques CE1/CE2 au format d'import de sesame.

Valide chaque question contre les règles de src/importer.rs avant d'écrire.
"""
import json
import sys

SUBJECT = "Mathématiques"

# (kind, statement, answers, explanation, difficulty)
# answers: number/exact -> "la bonne" (str) | single -> (correcte, [distracteurs]) | multi -> ([correctes], [fausses])
Q = []


def num(stmt, ans, expl, diff):
    Q.append({"kind": "number", "statement": stmt,
              "answers": [{"text": str(ans), "correct": True}],
              "explanation": expl, "difficulty": diff})


def exact(stmt, ans, expl, diff):
    Q.append({"kind": "exact", "statement": stmt,
              "answers": [{"text": ans, "correct": True}],
              "explanation": expl, "difficulty": diff})


def single(stmt, good, bads, expl, diff):
    ans = [{"text": good, "correct": True}] + [{"text": b, "correct": False} for b in bads]
    Q.append({"kind": "single", "statement": stmt, "answers": ans,
              "explanation": expl, "difficulty": diff})


def multi(stmt, goods, bads, expl, diff):
    ans = [{"text": g, "correct": True} for g in goods] + [{"text": b, "correct": False} for b in bads]
    Q.append({"kind": "multi", "statement": stmt, "answers": ans,
              "explanation": expl, "difficulty": diff})


# ---------------------------------------------------------------- numération
num("Quel est le chiffre des dizaines dans le nombre 274 ?", 7,
    "Dans 274 : 2 centaines, 7 dizaines, 4 unités. Le chiffre des dizaines est 7.", 2)
num("Combien y a-t-il de centaines dans le nombre 358 ?", 3,
    "358, c'est 3 centaines, 5 dizaines et 8 unités. Il y a 3 centaines.", 2)
num("Écris en chiffres : trois cent quarante-deux", 342,
    "Trois cents (300) + quarante (40) + deux (2) = 342.", 2)
num("Quel nombre vient juste après 199 ?", 200,
    "Après 199 vient 200. Les 10 dizaines font une centaine de plus.", 2)
num("Quel nombre vient juste avant 500 ?", 499,
    "Juste avant 500, il y a 499.", 2)
num("Dans le nombre 605, quel est le chiffre des unités ?", 5,
    "605 : 6 centaines, 0 dizaine, 5 unités. Le chiffre des unités est 5.", 1)
num("Combien font 10 dizaines ?", 100,
    "10 dizaines = 10 paquets de 10 = 100. C'est une centaine !", 2)
single("Quel est le plus grand de ces nombres ?", "470", ["407", "74", "47"],
       "On compare d'abord les centaines : 470 et 407 en ont 4, les autres aucune. Puis les dizaines : 7 > 0. Donc 470.", 2)
single("Quel est le plus petit de ces nombres ?", "80", ["89", "98", "108"],
       "108 a une centaine, il est le plus grand. Entre 80, 89 et 98, le plus petit est 80.", 1)
single("Quel nombre est compris entre 350 et 360 ?", "357", ["349", "362", "375"],
       "357 est bien plus grand que 350 et plus petit que 360. Les autres sont en dehors de cet intervalle.", 3)
multi("Quels nombres sont pairs ? (plusieurs réponses)", ["12", "30"], ["7", "25"],
      "Un nombre pair se termine par 0, 2, 4, 6 ou 8. Ici : 12 et 30.", 2)

# ------------------------------------------------------- addition/soustraction
single("Combien font 8 + 7 ?", "15", ["14", "16", "13"],
       "8 + 7 = 15. Astuce : 8 + 2 = 10, puis il reste 5 à ajouter → 15.", 1)
num("Julie avait 13 images. Elle en donne 6 à son frère. Combien lui en reste-t-il ?", 7,
    "Elle en donne, donc on soustrait : 13 - 6 = 7 images.", 1)
single("45 + 27 = ?", "72", ["62", "73", "82"],
       "45 + 27 : les unités 5 + 7 = 12 (je pose 2, je retiens 1), les dizaines 4 + 2 + 1 = 7. Résultat : 72. Attention à ne pas oublier la retenue !", 3)
num("68 + 15 = ?", 83,
    "68 + 15 : 8 + 5 = 13 (je retiens 1), puis 6 + 1 + 1 = 8. Résultat : 83.", 3)
num("56 - 24 = ?", 32,
    "56 - 24 : 6 - 4 = 2 pour les unités, 5 - 2 = 3 pour les dizaines. Résultat : 32.", 2)
single("100 - 37 = ?", "63", ["73", "67", "53"],
       "100 - 37 = 63. Astuce : de 37 à 40 il y a 3, de 40 à 100 il y a 60. 3 + 60 = 63.", 3)
num("234 + 100 = ?", 334,
    "Ajouter 100, c'est ajouter 1 centaine : 234 → 334. Les dizaines et les unités ne changent pas.", 2)
num("Combien manque-t-il à 60 pour aller jusqu'à 100 ?", 40,
    "60 + 40 = 100. Il manque donc 40.", 2)
num("27 + 30 = ?", 57,
    "27 + 30 : on ajoute 3 dizaines. 27 → 37 → 47 → 57.", 2)
num("150 - 50 = ?", 100,
    "150 - 50 = 100. On enlève 5 dizaines.", 2)
num("Complète : 7 + ? = 15", 8,
    "7 + 8 = 15. On cherche ce qu'il faut ajouter à 7 pour atteindre 15 : c'est 8.", 2)

# ---------------------------------------------------------------- multiplication
num("4 × 3 = ?", 12,
    "4 × 3, c'est 3 + 3 + 3 + 3 = 12.", 2)
num("2 × 8 = ?", 16,
    "2 × 8 = 16. C'est le double de 8.", 2)
num("6 × 10 = ?", 60,
    "Multiplier par 10, c'est ajouter un zéro : 6 × 10 = 60.", 2)
single("5 × 5 = ?", "25", ["20", "30", "10"],
       "5 × 5 = 25. C'est dans la table de 5.", 2)
single("9 × 3 = ?", "27", ["12", "24", "30"],
       "9 × 3 = 27. Astuce : 10 × 3 = 30, puis on enlève un 3 → 27. Attention : 9 + 3 = 12, ce n'est pas la même chose !", 3)
num("8 × 4 = ?", 32,
    "8 × 4 = 32. C'est le double de 8 × 2 = 16.", 3)
num("7 × 5 = ?", 35,
    "7 × 5 = 35. Dans la table de 5, les résultats finissent par 0 ou 5.", 3)
num("3 × 100 = ?", 300,
    "Multiplier par 100, c'est ajouter deux zéros : 3 × 100 = 300.", 3)
num("Quel est le double de 14 ?", 28,
    "Le double, c'est deux fois : 14 + 14 = 28.", 2)
single("Quelle est la moitié de 18 ?", "9", ["36", "8", "10"],
       "La moitié, c'est partager en deux parts égales : 9 + 9 = 18, donc la moitié de 18 est 9. 36, c'est le DOUBLE, pas la moitié !", 2)
num("Quel est le triple de 5 ?", 15,
    "Le triple, c'est trois fois : 5 × 3 = 15.", 3)
multi("Quels résultats sont dans la table de 5 ? (plusieurs réponses)", ["20", "35"], ["22", "18"],
      "Dans la table de 5, les résultats se terminent par 0 ou 5 : 20 (5 × 4) et 35 (5 × 7).", 3)

# --------------------------------------------------------------------- division
num("On partage 12 bonbons entre 3 enfants. Combien de bonbons chacun ?", 4,
    "12 ÷ 3 = 4. Chaque enfant reçoit 4 bonbons, car 3 × 4 = 12.", 2)
num("20 ÷ 4 = ?", 5,
    "20 ÷ 4 = 5, car 4 × 5 = 20.", 3)
single("Combien de fois 6 tient-il dans 30 ?", "5", ["6", "4", "24"],
       "6 × 5 = 30, donc 6 tient 5 fois dans 30.", 3)
num("On range 24 œufs dans des boîtes de 6. Combien de boîtes ?", 4,
    "24 ÷ 6 = 4, car 6 × 4 = 24. Il faut 4 boîtes.", 3)

# --------------------------------------------------------------------- problèmes
num("Léa a 35 billes. Elle en gagne 18. Combien de billes a-t-elle maintenant ?", 53,
    "Elle en gagne, donc on additionne : 35 + 18 = 53 billes.", 3)
num("Tom avait 50 €. Il dépense 22 €. Combien lui reste-t-il ?", 28,
    "Il dépense, donc on soustrait : 50 - 22 = 28 €.", 3)
num("Un paquet contient 8 gâteaux. Combien de gâteaux dans 3 paquets ?", 24,
    "3 paquets de 8 : 8 × 3 = 24 gâteaux.", 3)
num("Une place de cinéma coûte 7 €. Combien coûtent 4 places ?", 28,
    "4 places à 7 € : 7 × 4 = 28 €.", 3)
num("Dans la salle, il y a 5 rangées de 6 chaises. Combien de chaises en tout ?", 30,
    "5 rangées × 6 chaises = 30 chaises.", 3)
single("Un livre a 120 pages. J'en ai lu 45. Combien de pages me reste-t-il à lire ?", "75",
       ["165", "85", "65"],
       "« Il reste » : on soustrait. 120 - 45 = 75 pages. Si tu as trouvé 165, tu as additionné au lieu de soustraire.", 3)
num("Marie achète un cahier à 3 € et un stylo à 2 €. Elle paie avec un billet de 10 €. Combien lui rend-on ?", 5,
    "Elle dépense 3 + 2 = 5 €. On lui rend 10 - 5 = 5 €.", 3)
single("Dans le bus il y a 28 personnes. À l'arrêt, 9 descendent et 5 montent. Combien de personnes y a-t-il maintenant ?",
       "24", ["32", "42", "14"],
       "Deux étapes : 28 - 9 = 19 (ceux qui descendent partent), puis 19 + 5 = 24 (ceux qui montent arrivent).", 3)
num("J'ai un billet de 10 € et une pièce de 2 €. Combien d'argent ai-je ?", 12,
    "10 + 2 = 12 €.", 2)
num("Combien faut-il de pièces de 2 € pour faire 10 € ?", 5,
    "2 × 5 = 10, il faut donc 5 pièces de 2 €.", 3)

# ---------------------------------------------------------------------- mesures
num("Combien y a-t-il de centimètres dans 1 mètre ?", 100,
    "1 mètre = 100 centimètres (1 m = 100 cm).", 2)
num("Combien y a-t-il de grammes dans 1 kilogramme ?", 1000,
    "1 kilogramme = 1000 grammes (1 kg = 1000 g).", 2)
num("2 mètres, c'est combien de centimètres ?", 200,
    "1 m = 100 cm, donc 2 m = 2 × 100 = 200 cm.", 3)
single("1 litre, c'est combien de centilitres ?", "100 cL", ["10 cL", "1000 cL", "60 cL"],
       "1 litre = 100 centilitres (1 L = 100 cL), comme 1 mètre = 100 centimètres. Le préfixe « centi » veut dire centième.", 3)
num("Combien y a-t-il de minutes dans 1 heure ?", 60,
    "1 heure = 60 minutes.", 1)
num("Combien y a-t-il de minutes dans une demi-heure ?", 30,
    "Une demi-heure, c'est la moitié de 60 minutes : 30 minutes.", 2)
num("Combien y a-t-il d'heures dans une demi-journée ?", 12,
    "Une journée dure 24 heures. La moitié de 24, c'est 12 heures.", 3)
num("Combien y a-t-il de jours dans deux semaines ?", 14,
    "Une semaine a 7 jours, donc deux semaines font 7 × 2 = 14 jours.", 2)
num("Combien y a-t-il de mois dans une demi-année ?", 6,
    "Une année compte 12 mois. La moitié de 12, c'est 6 mois.", 3)
single("Quelle unité utilise-t-on pour mesurer la longueur d'un crayon ?", "le centimètre",
       ["le litre", "le kilogramme", "l'heure"],
       "Une longueur se mesure en centimètres (cm). Le litre mesure un liquide, le kilogramme une masse.", 2)
single("Quel objet pèse environ 1 kilogramme ?", "un paquet de sucre",
       ["une plume", "une voiture", "un crayon"],
       "Un paquet de sucre pèse environ 1 kg. Une plume est bien plus légère, une voiture bien plus lourde.", 2)

# -------------------------------------------------------------------- géométrie
num("Je dessine un carré. Combien de traits dois-je tracer ?", 4,
    "Le carré a 4 côtés, tous de la même longueur : il faut donc tracer 4 traits.", 1)
num("Combien de sommets (de coins) a un triangle ?", 3,
    "Le triangle a 3 côtés et 3 sommets. « Tri » veut dire trois !", 2)
num("Combien de sommets a un rectangle ?", 4,
    "Le rectangle a 4 sommets (les coins) et 4 côtés.", 2)
single("Combien de faces a un cube ?", "6", ["4", "8", "12"],
       "Le cube a 6 faces carrées, comme un dé. Il a aussi 8 sommets et 12 arêtes — attention à ne pas confondre !", 3)
num("Combien d'angles droits y a-t-il dans un rectangle ?", 4,
    "Le rectangle a 4 angles droits, un à chaque coin.", 3)
num("Un rectangle mesure 5 cm de long et 3 cm de large. Quel est son périmètre, en cm ?", 16,
    "Le périmètre, c'est le tour de la figure : 5 + 3 + 5 + 3 = 16 cm. On additionne les 4 côtés.", 3)
single("Quelle figure n'a aucun côté droit ?", "le cercle", ["le carré", "le triangle", "le rectangle"],
       "Le cercle est une ligne courbe : il n'a pas de côté droit ni de sommet.", 2)
single("Quelle figure a 4 côtés de la même longueur et 4 angles droits ?", "le carré",
       ["le rectangle", "le cercle", "le triangle"],
       "Le carré : 4 côtés égaux et 4 angles droits. Le rectangle a 4 angles droits mais ses côtés ne sont pas tous égaux.", 2)
multi("Quelles figures ont 4 côtés ? (plusieurs réponses)", ["le carré", "le rectangle"],
      ["le triangle", "le cercle"],
      "Le carré et le rectangle ont 4 côtés. Le triangle en a 3, le cercle aucun.", 2)
exact("Comment s'appelle une figure qui a 3 côtés ? (écris un seul mot, sans article)", "triangle",
      "Une figure à 3 côtés s'appelle un triangle.", 2)

# ------------------------------------------------------------------------ heure
single("La petite aiguille est sur le 3 et la grande aiguille sur le 12. Quelle heure est-il ?",
       "3 heures", ["12 heures", "3 heures et demie", "midi et quart"],
       "La petite aiguille donne l'heure (3), la grande sur le 12 signifie « pile ». Il est 3 heures.", 2)
num("Il est 8 heures. Dans 2 heures, quelle heure sera-t-il ? (réponds par un nombre)", 10,
    "8 + 2 = 10. Il sera 10 heures.", 2)

# --------------------------------------------------------------- suites / calcul
num("Continue la suite : 2, 4, 6, 8, ?", 10,
    "On ajoute 2 à chaque fois : après 8 vient 10.", 1)
num("Continue la suite : 5, 10, 15, 20, ?", 25,
    "On avance de 5 en 5 : après 20 vient 25.", 2)
single("Continue la suite : 100, 90, 80, 70, ?", "60", ["50", "65", "75"],
       "On recule de 10 en 10 : après 70 vient 60.", 2)
num("Combien font 20 + 20 + 20 ?", 60,
    "20 + 20 + 20 = 60. C'est aussi 20 × 3.", 2)

# ---------------------------------------------------------------------- validation
errors = []
seen = set()
for i, q in enumerate(Q):
    tag = f"[{i}] {q['statement'][:45]}"
    if q["statement"] in seen:
        errors.append(f"{tag} : ÉNONCÉ DUPLIQUÉ")
    seen.add(q["statement"])
    if not (1 <= q["difficulty"] <= 5):
        errors.append(f"{tag} : difficulté hors [1,5]")
    correct = sum(1 for a in q["answers"] if a["correct"])
    texts = [a["text"].strip().lower() for a in q["answers"]]
    if len(texts) != len(set(texts)):
        errors.append(f"{tag} : réponses dupliquées")
    if any(not a["text"].strip() for a in q["answers"]):
        errors.append(f"{tag} : réponse vide")
    if q["kind"] in ("number", "exact"):
        if len(q["answers"]) != 1 or not q["answers"][0]["correct"]:
            errors.append(f"{tag} : {q['kind']} exige exactement 1 réponse correcte")
        if q["kind"] == "number":
            try:
                float(q["answers"][0]["text"].replace(",", "."))
            except ValueError:
                errors.append(f"{tag} : 'number' dont la réponse n'est pas un nombre")
    elif q["kind"] == "single":
        if len(q["answers"]) < 2:
            errors.append(f"{tag} : single avec moins de 2 réponses")
        if correct != 1:
            errors.append(f"{tag} : single avec {correct} réponses correctes")
    elif q["kind"] == "multi":
        if correct < 1 or correct == len(q["answers"]):
            errors.append(f"{tag} : multi mal formé ({correct}/{len(q['answers'])} correctes)")
    else:
        errors.append(f"{tag} : kind invalide '{q['kind']}'")

if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)

for q in Q:
    q["subject"] = SUBJECT

out = {
    "subjects": [{"name": SUBJECT, "weight": 1.0}],
    "questions": [
        {"subject": q["subject"], "kind": q["kind"], "statement": q["statement"],
         "answers": q["answers"], "explanation": q["explanation"], "difficulty": q["difficulty"]}
        for q in Q
    ],
}

path = "data/questions_maths_ce1_ce2.json"
with open(path, "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
    f.write("\n")

from collections import Counter
print(f"OK — {len(Q)} questions écrites dans {path}")
print("  kinds      :", dict(Counter(q["kind"] for q in Q)))
print("  difficultés:", dict(sorted(Counter(q["difficulty"] for q in Q).items())))
