-- luanti-gate: schema initial
-- Convention: integers are stored as INTEGER (Unix epoch seconds for timestamps),
-- booleans as 0/1 with CHECK, floats as REAL.

PRAGMA foreign_keys = ON;

CREATE TABLE subjects (
    id      INTEGER PRIMARY KEY,
    name    TEXT    NOT NULL UNIQUE,
    weight  REAL    NOT NULL DEFAULT 1.0 CHECK (weight > 0)
);

CREATE TABLE questions (
    id          INTEGER PRIMARY KEY,
    subject_id  INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK (kind IN ('single', 'multi')),
    statement   TEXT    NOT NULL,
    explanation TEXT,
    created_at  INTEGER NOT NULL
);

CREATE INDEX idx_questions_subject ON questions(subject_id);

CREATE TABLE answers (
    id          INTEGER PRIMARY KEY,
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    text        TEXT    NOT NULL,
    is_correct  INTEGER NOT NULL CHECK (is_correct IN (0, 1))
);

CREATE INDEX idx_answers_question ON answers(question_id);

CREATE TABLE attempts (
    id          INTEGER PRIMARY KEY,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    score_pct   REAL    CHECK (score_pct IS NULL OR (score_pct >= 0 AND score_pct <= 100)),
    passed      INTEGER NOT NULL DEFAULT 0 CHECK (passed IN (0, 1))
);

CREATE INDEX idx_attempts_started ON attempts(started_at);

CREATE TABLE attempt_questions (
    attempt_id          INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    question_id         INTEGER NOT NULL REFERENCES questions(id),
    answered_correctly  INTEGER NOT NULL CHECK (answered_correctly IN (0, 1)),
    PRIMARY KEY (attempt_id, question_id)
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Defaults runtime — modificables desde /admin sin reiniciar.
INSERT INTO settings (key, value) VALUES
    ('questions_per_test',     '10'),
    ('pass_threshold_pct',     '70'),
    ('kill_interval_minutes',  '30');
-- admin_password_hash NO se inserta — el primer arranque del panel admin
-- obliga a definir una contraseña.

-- Asignaturas iniciales en francés (peso uniforme).
INSERT INTO subjects (name, weight) VALUES
    ('mathématiques', 1.0),
    ('biologie',      1.0),
    ('compréhension', 1.0),
    ('conjugaison',   1.0);
