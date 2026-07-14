-- Deux nouveaux types de question : 'exact' (réponse écrite) et 'number'
-- (réponse numérique). Dans les deux cas la bonne réponse est STOCKÉE dans
-- `answers` (une seule ligne, is_correct = 1) — elle n'est jamais calculée à
-- partir de l'énoncé : un énoncé n'est pas toujours une expression
-- (« 0, 1, 2, 3, ... ? »), et un bug d'évaluation refuserait l'ordinateur à un
-- enfant qui a juste. La différence entre les deux types est la COMPARAISON :
--   exact  → texte, extrémités rognées, casse ignorée (les accents comptent)
--   number → numérique, « 08 » == « 8 » == « 8,0 » == « +8 »
--
-- ATTENTION — SQLite ne sait pas modifier une contrainte CHECK : il faut
-- reconstruire la table. Or `questions` est référencée par `answers`
-- (ON DELETE CASCADE) : un `DROP TABLE questions` avec les clés étrangères
-- ACTIVES déclenche un DELETE implicite qui ferait CASCADER la suppression de
-- TOUTES les réponses. Cette migration N'EST SÛRE que parce que db.rs les
-- désactive sur la connexion du migrateur (voir `run_migrations`) — et ça ne
-- peut se faire que là : le PRAGMA est un no-op dans une transaction, et
-- sqlx-sqlite enveloppe chaque migration dans une transaction.

-- ===== questions ============================================================

CREATE TABLE questions_new (
    id          INTEGER PRIMARY KEY,
    subject_id  INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK (kind IN ('single', 'multi', 'exact', 'number')),
    statement   TEXT    NOT NULL,
    explanation TEXT,
    created_at  INTEGER NOT NULL,
    difficulty  INTEGER NOT NULL DEFAULT 3 CHECK (difficulty BETWEEN 1 AND 5)
);

INSERT INTO questions_new (id, subject_id, kind, statement, explanation, created_at, difficulty)
SELECT id, subject_id, kind, statement, explanation, created_at, difficulty FROM questions;

DROP TABLE questions;
ALTER TABLE questions_new RENAME TO questions;

CREATE INDEX idx_questions_subject    ON questions(subject_id);
CREATE INDEX idx_questions_difficulty ON questions(difficulty);

-- ===== attempt_answers ======================================================
-- Même CHECK à élargir, plus une colonne : ce que l'enfant a ÉCRIT. Pour
-- 'single'/'multi' l'info est déjà là (was_chosen sur chaque option) ; pour
-- 'exact'/'number' elle serait perdue à jamais — le parent verrait « raté »
-- sans jamais savoir si l'enfant a écrit 84 au lieu de 85, ou n'importe quoi.
-- NULL pour les types à choix.

CREATE TABLE attempt_answers_new (
    id                    INTEGER PRIMARY KEY,
    attempt_id            INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    question_id           INTEGER NOT NULL,
    kind_snapshot         TEXT    NOT NULL CHECK (kind_snapshot IN ('single', 'multi', 'exact', 'number')),
    statement_snapshot    TEXT    NOT NULL,
    answer_id             INTEGER NOT NULL,
    answer_text_snapshot  TEXT    NOT NULL,
    given_text_snapshot   TEXT,
    was_chosen            INTEGER NOT NULL CHECK (was_chosen IN (0, 1)),
    is_correct            INTEGER NOT NULL CHECK (is_correct IN (0, 1))
);

INSERT INTO attempt_answers_new
    (id, attempt_id, question_id, kind_snapshot, statement_snapshot,
     answer_id, answer_text_snapshot, given_text_snapshot, was_chosen, is_correct)
SELECT
     id, attempt_id, question_id, kind_snapshot, statement_snapshot,
     answer_id, answer_text_snapshot, NULL, was_chosen, is_correct
FROM attempt_answers;

DROP TABLE attempt_answers;
ALTER TABLE attempt_answers_new RENAME TO attempt_answers;

CREATE INDEX idx_attempt_answers_attempt  ON attempt_answers(attempt_id);
CREATE INDEX idx_attempt_answers_question ON attempt_answers(attempt_id, question_id);
