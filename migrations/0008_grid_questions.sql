-- Deux nouveaux types de question : 'grid_cells' et 'grid_lines' — l'exercice
-- scolaire « reproduis le dessin sur la grille vierge ». Le modèle s'affiche à
-- gauche, la grille vierge à droite, et la réponse n'est juste que si le dessin
-- correspond à 100 %.
--
-- OÙ VIT LA FIGURE — et pourquoi pas dans une table à part.
-- Dans l'unique ligne `answers` (is_correct = 1), en TEXTE autodescriptif :
--     grid_cells → « 8x8:c=0,3;1,3;1,4 »        (cases coloriées, r,c)
--     grid_lines → « 8x8:e=0,0-0,1;1,1-2,2 »    (segments, extrémités du treillis)
-- Une table dédiée aurait été mieux typée, mais elle aurait cassé deux choses
-- qui marchent déjà : la bonne réponse vit dans `answers` (cf. 0006), et
-- `attempt_answers` en garde un INSTANTANÉ (`answer_text_snapshot`) pour que
-- l'historique survive à l'édition ou à la suppression de la question. Le
-- pendant côté enfant — ce qu'il a dessiné — a déjà sa colonne : c'est
-- `given_text_snapshot`, ajoutée en 0006 pour les réponses écrites. Ces deux
-- types-là n'ont donc besoin d'AUCUNE colonne nouvelle.
--
-- CE QUE CE TYPE CHANGE, ET QU'IL FAUT DIRE TOUT HAUT.
-- 0006 posait en règle que la bonne réponse ne doit JAMAIS atteindre le client :
-- un Ctrl+U et l'enfant lit le résultat. Ici c'est l'inverse, par nature — la
-- figure à reproduire EST le modèle affiché. Il n'y a rien à protéger :
-- l'exercice n'est pas « devine », il est « recopie exactement ». `quiz.rs`
-- sépare donc désormais deux questions qui se confondaient : « la réponse se
-- tape-t-elle ? » (is_free_input) et « la réponse est-elle secrète ? »
-- (answer_is_secret).
--
-- ATTENTION — même piège qu'en 0006 : SQLite ne sait pas modifier une contrainte
-- CHECK, il faut reconstruire la table. `DROP TABLE questions` avec les clés
-- étrangères ACTIVES ferait CASCADER la suppression de toutes les `answers`.
-- Cette migration n'est sûre que parce que db.rs les désactive sur la connexion
-- du migrateur (voir `run_migrations`).

-- ===== questions ============================================================

CREATE TABLE questions_new (
    id          INTEGER PRIMARY KEY,
    subject_id  INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK (kind IN ('single', 'multi', 'exact', 'number',
                                                 'grid_cells', 'grid_lines')),
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
-- `kind_snapshot` porte le même CHECK : sans ça, enregistrer une tentative sur
-- une question grille échouerait à l'écriture — l'enfant aurait répondu, et
-- l'application planterait au moment de le noter.

CREATE TABLE attempt_answers_new (
    id                    INTEGER PRIMARY KEY,
    attempt_id            INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    question_id           INTEGER NOT NULL,
    kind_snapshot         TEXT    NOT NULL CHECK (kind_snapshot IN ('single', 'multi', 'exact', 'number',
                                                                    'grid_cells', 'grid_lines')),
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
     answer_id, answer_text_snapshot, given_text_snapshot, was_chosen, is_correct
FROM attempt_answers;

DROP TABLE attempt_answers;
ALTER TABLE attempt_answers_new RENAME TO attempt_answers;

CREATE INDEX idx_attempt_answers_attempt  ON attempt_answers(attempt_id);
CREATE INDEX idx_attempt_answers_question ON attempt_answers(attempt_id, question_id);

-- ===== matière ==============================================================
-- Matière propre, et pas un ajout à « mathématiques » : c'est ce qui permet de
-- lui donner un poids FORT pour l'enfant qui en a besoin et un poids faible
-- pour les autres, via `child_subject_weights`. Les enfants déjà créés n'ont
-- pas de ligne pour elle — c'est prévu : `pick_questions` retombe (COALESCE)
-- sur les valeurs par défaut ci-dessous tant que le parent n'a rien réglé.

INSERT OR IGNORE INTO subjects (name, weight, enabled) VALUES
    ('Espace et Géométrie', 1.0, 1);
