-- Poids et activation des matières PAR ENFANT.
--
-- Avant : `subjects.weight` / `subjects.enabled` étaient globaux — une seule
-- valeur partagée par tous les enfants. Désormais chaque enfant possède son
-- propre réglage, matière par matière.
--
-- `subjects.weight` / `subjects.enabled` NE disparaissent pas : ils deviennent
-- les VALEURS PAR DÉFAUT, avec deux rôles —
--   1. le gabarit hérité par tout nouvel enfant (semé à la création) ;
--   2. le repli (COALESCE au moment de la lecture) pour une matière ajoutée
--      APRÈS l'enfant, qui n'a donc pas encore de ligne ici.
-- La ligne enfant×matière, quand elle existe, fait toujours foi.

CREATE TABLE child_subject_weights (
    child_id   INTEGER NOT NULL REFERENCES children(id) ON DELETE CASCADE,
    subject_id INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    weight     REAL    NOT NULL DEFAULT 1.0 CHECK (weight > 0),
    enabled    INTEGER NOT NULL DEFAULT 1  CHECK (enabled IN (0, 1)),
    PRIMARY KEY (child_id, subject_id)
);

-- Backfill : chaque enfant existant hérite des valeurs globales d'aujourd'hui,
-- pour que le comportement soit rigoureusement identique juste après migration.
INSERT INTO child_subject_weights (child_id, subject_id, weight, enabled)
SELECT c.id, s.id, s.weight, s.enabled
FROM children c CROSS JOIN subjects s;
