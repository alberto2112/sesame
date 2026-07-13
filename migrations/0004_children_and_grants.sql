-- Pivote : le portail ne bloque plus un jeu, il bloque l'ordinateur.
--
-- Nouveau modèle : un ENFANT passe un contrôle et obtient une CONCESSION de
-- temps (grant). Le temps consommé est comptabilisé dans un grand livre
-- journalier (daily_usage) pour que le budget quotidien survive aux
-- redémarrages et ne puisse pas être « farmé » en enchaînant les contrôles.

PRAGMA foreign_keys = ON;

-- ===== Enfants ==============================================================

CREATE TABLE children (
    id                     INTEGER PRIMARY KEY,
    name                   TEXT    NOT NULL UNIQUE,
    -- Un enfant de 6 ans ne lit pas encore : il reconnaît son emoji.
    avatar                 TEXT    NOT NULL DEFAULT '🙂',
    -- NULL = pas de code. Réservé : permet d'exiger un code à 4 chiffres si un
    -- jour l'aîné se fait passer pour le cadet pour avoir un contrôle facile.
    pin_hash               TEXT,
    enabled                INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    position               INTEGER NOT NULL DEFAULT 0,

    -- Contrôle (surcharge les valeurs globales de `settings`).
    difficulty_min         INTEGER NOT NULL DEFAULT 1  CHECK (difficulty_min BETWEEN 1 AND 5),
    difficulty_max         INTEGER NOT NULL DEFAULT 5  CHECK (difficulty_max BETWEEN 1 AND 5),
    questions_per_test     INTEGER NOT NULL DEFAULT 10 CHECK (questions_per_test > 0),
    pass_threshold_pct     REAL    NOT NULL DEFAULT 70 CHECK (pass_threshold_pct BETWEEN 0 AND 100),

    -- Temps.
    session_minutes        INTEGER NOT NULL DEFAULT 30  CHECK (session_minutes > 0),
    daily_budget_minutes   INTEGER NOT NULL DEFAULT 60  CHECK (daily_budget_minutes >= 0),
    weekend_budget_minutes INTEGER NOT NULL DEFAULT 120 CHECK (weekend_budget_minutes >= 0),
    exam_cooldown_minutes  INTEGER NOT NULL DEFAULT 0   CHECK (exam_cooldown_minutes >= 0),

    CHECK (difficulty_min <= difficulty_max)
);

-- ===== Plages horaires ======================================================
-- Plusieurs lignes par jour = plusieurs fenêtres (matin ET après-midi).
-- Hors fenêtre : « C'est l'heure de dormir ». Câblé en phase 3.

CREATE TABLE schedules (
    id        INTEGER PRIMARY KEY,
    child_id  INTEGER NOT NULL REFERENCES children(id) ON DELETE CASCADE,
    weekday   INTEGER NOT NULL CHECK (weekday BETWEEN 0 AND 6),  -- 0 = lundi
    start_min INTEGER NOT NULL CHECK (start_min BETWEEN 0 AND 1440),
    end_min   INTEGER NOT NULL CHECK (end_min BETWEEN 0 AND 1440),
    CHECK (start_min < end_min)
);

CREATE INDEX idx_schedules_child ON schedules(child_id, weekday);

-- ===== Grand livre de consommation ==========================================
-- UNE ligne par enfant et par jour. Le heartbeat l'incrémente.
--
-- Volontairement un compteur de secondes CONSOMMÉES, et non une heure
-- d'expiration : le passage de minuit tombe naturellement dans le bon jour, le
-- redémarrage ne réinitialise rien, et changer l'horloge du système ne donne
-- pas de minutes gratuites (le temps écoulé est mesuré de façon MONOTONE côté
-- client, le serveur ne fait qu'additionner).

CREATE TABLE daily_usage (
    child_id      INTEGER NOT NULL REFERENCES children(id) ON DELETE CASCADE,
    day           TEXT    NOT NULL,                       -- 'YYYY-MM-DD' local
    consumed_secs INTEGER NOT NULL DEFAULT 0 CHECK (consumed_secs >= 0),
    PRIMARY KEY (child_id, day)
);

-- ===== Concessions ==========================================================

CREATE TABLE grants (
    id            INTEGER PRIMARY KEY,
    child_id      INTEGER NOT NULL REFERENCES children(id) ON DELETE CASCADE,
    -- NULL = concession manuelle du parent (bouton de secours).
    attempt_id    INTEGER REFERENCES attempts(id) ON DELETE SET NULL,
    granted_at    INTEGER NOT NULL,
    minutes       INTEGER NOT NULL CHECK (minutes > 0),
    consumed_secs INTEGER NOT NULL DEFAULT 0 CHECK (consumed_secs >= 0),
    closed_at     INTEGER                                 -- NULL = concession vivante
);

CREATE INDEX idx_grants_child_open ON grants(child_id, closed_at);
-- Un contrôle réussi ne peut être échangé qu'une seule fois contre du temps.
CREATE UNIQUE INDEX idx_grants_attempt ON grants(attempt_id) WHERE attempt_id IS NOT NULL;

-- ===== Colonnes ajoutées ====================================================

ALTER TABLE attempts  ADD COLUMN child_id   INTEGER REFERENCES children(id);
ALTER TABLE questions ADD COLUMN difficulty INTEGER NOT NULL DEFAULT 3
                                            CHECK (difficulty BETWEEN 1 AND 5);

CREATE INDEX idx_attempts_child ON attempts(child_id);
CREATE INDEX idx_questions_difficulty ON questions(difficulty);

-- ===== Données =============================================================

-- Enfant par défaut : la base existante n'en avait aucun.
INSERT INTO children (name, avatar) VALUES ('Enfant', '🙂');
UPDATE attempts SET child_id = (SELECT id FROM children LIMIT 1) WHERE child_id IS NULL;

-- Le jeu n'existe plus : l'intervalle ne « tue » plus rien, il borne une session.
UPDATE settings SET key = 'session_minutes' WHERE key = 'kill_interval_minutes';

-- Ce qui se passe quand le temps est écoulé :
--   'overlay' → une fenêtre recouvre le bureau, l'enfant peut repasser un contrôle
--   'logout'  → avertissement de 60 s puis fermeture de session
INSERT INTO settings (key, value) VALUES ('lock_mode', 'overlay')
    ON CONFLICT(key) DO NOTHING;
