-- Concessions « parent » : le bouton de secours de /admin/children.
--
-- bypass = 1 → la concession ignore le budget quotidien ET les plages
-- horaires. Sans ce drapeau, le bouton de secours serait inutile PRÉCISÉMENT
-- quand on en a besoin : budget épuisé ou couvre-feu, le moteur rabotait la
-- concession à 0 seconde et la refermait aussitôt.
--
-- La consommation reste comptabilisée dans daily_usage (les statistiques
-- disent la vérité) ; elle n'est simplement plus un plafond pour CETTE
-- concession.

ALTER TABLE grants ADD COLUMN bypass INTEGER NOT NULL DEFAULT 0 CHECK (bypass IN (0, 1));
