-- Sesiones del panel admin (single user — el padre).
-- token: 32 bytes random base64-url (43 chars sin padding).
CREATE TABLE admin_sessions (
    token       TEXT    PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);

CREATE INDEX idx_admin_sessions_expires ON admin_sessions(expires_at);

-- Detalle granular de cada respuesta de cada attempt, con snapshot de texto.
-- Snapshot deliberado: el historial sobrevive a edición/borrado de preguntas.
-- El (question_id, answer_id) se conservan como referencia informativa, sin FK.
CREATE TABLE attempt_answers (
    id                    INTEGER PRIMARY KEY,
    attempt_id            INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    question_id           INTEGER NOT NULL,
    kind_snapshot         TEXT    NOT NULL CHECK (kind_snapshot IN ('single', 'multi')),
    statement_snapshot    TEXT    NOT NULL,
    answer_id             INTEGER NOT NULL,
    answer_text_snapshot  TEXT    NOT NULL,
    was_chosen            INTEGER NOT NULL CHECK (was_chosen IN (0, 1)),
    is_correct            INTEGER NOT NULL CHECK (is_correct IN (0, 1))
);

CREATE INDEX idx_attempt_answers_attempt ON attempt_answers(attempt_id);
CREATE INDEX idx_attempt_answers_question ON attempt_answers(attempt_id, question_id);
