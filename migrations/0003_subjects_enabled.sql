-- Ajoute un interrupteur d'activation par matière.
-- Une matière désactivée (enabled = 0) est ignorée par le sélecteur de
-- questions, mais conserve ses questions et son poids.
ALTER TABLE subjects ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
