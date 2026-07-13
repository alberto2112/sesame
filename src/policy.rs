//! Moteur de décision : *cet enfant, maintenant, peut-il utiliser l'ordinateur ?*
//!
//! C'est la SEULE source de vérité. Le kiosque de démarrage, la fenêtre de
//! verrouillage, le minuteur et l'API appellent tous [`evaluate`]. Si une
//! nouvelle condition apparaît (plage horaire, délai entre contrôles, jour
//! férié…), elle se branche ici et partout ailleurs, rien ne bouge.
//!
//! ## Le temps
//!
//! Le serveur ne calcule JAMAIS une durée écoulée à partir de l'horloge du
//! système : il additionne des secondes que le minuteur lui envoie, mesurées de
//! façon monotone. Changer l'heure de la machine ne donne donc pas de minutes
//! gratuites. `granted_at` n'est qu'informatif.

use anyhow::{Context, Result};
use chrono::{Datelike, Local, Weekday};
use serde::Serialize;
use sqlx::SqlitePool;

// ===== Enfant ===============================================================

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Child {
    pub id: i64,
    pub name: String,
    pub avatar: String,
    pub enabled: i64,

    pub difficulty_min: i64,
    pub difficulty_max: i64,
    pub questions_per_test: i64,
    pub pass_threshold_pct: f64,

    pub session_minutes: i64,
    pub daily_budget_minutes: i64,
    pub weekend_budget_minutes: i64,
    pub exam_cooldown_minutes: i64,
}

const CHILD_COLUMNS: &str = "id, name, avatar, enabled, difficulty_min, difficulty_max, \
     questions_per_test, pass_threshold_pct, session_minutes, daily_budget_minutes, \
     weekend_budget_minutes, exam_cooldown_minutes";

pub async fn load_child(pool: &SqlitePool, id: i64) -> Result<Option<Child>> {
    let sql = format!("SELECT {CHILD_COLUMNS} FROM children WHERE id = ? AND enabled = 1");
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}

pub async fn list_children(pool: &SqlitePool) -> Result<Vec<Child>> {
    let sql =
        format!("SELECT {CHILD_COLUMNS} FROM children WHERE enabled = 1 ORDER BY position, id");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Enfant utilisé pour les outils console (preview) : le premier actif.
pub async fn default_child(pool: &SqlitePool) -> Result<Child> {
    let sql =
        format!("SELECT {CHILD_COLUMNS} FROM children WHERE enabled = 1 ORDER BY position, id LIMIT 1");
    sqlx::query_as(&sql)
        .fetch_optional(pool)
        .await?
        .context("aucun enfant actif dans la base — crée-en un depuis /admin")
}

impl Child {
    /// Budget du jour, en secondes. Samedi et dimanche ont le leur.
    fn budget_secs_today(&self) -> i64 {
        let minutes = match Local::now().weekday() {
            Weekday::Sat | Weekday::Sun => self.weekend_budget_minutes,
            _ => self.daily_budget_minutes,
        };
        minutes * 60
    }
}

// ===== Décision =============================================================

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GateDecision {
    /// L'enfant a du temps vivant : l'ordinateur est à lui.
    Granted { remaining_secs: i64 },

    /// Pas de temps, mais il peut le gagner : un contrôle l'attend.
    ExamAvailable {
        questions: i64,
        threshold_pct: f64,
        /// Ce qu'un contrôle réussi rapporterait MAINTENANT. Borné par ce qui
        /// reste du budget quotidien : réussir dix contrôles ne crée pas de
        /// minutes qui n'existent pas.
        max_grant_minutes: i64,
    },

    /// Il ne peut même pas essayer. Le motif EST l'écran affiché.
    Blocked { reason: BlockReason },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlockReason {
    /// Budget du jour épuisé. Un contrôle de plus n'y changera rien.
    DailyBudgetSpent { used_min: i64, budget_min: i64 },
    // Phase 3 : OutsideSchedule { next_window_at }, Cooldown { until }
}

// ===== Évaluation ===========================================================

pub async fn evaluate(pool: &SqlitePool, child: &Child) -> Result<GateDecision> {
    let budget_secs = child.budget_secs_today();
    let used_secs = consumed_today(pool, child.id).await?;
    let budget_left = (budget_secs - used_secs).max(0);

    // Une concession vivante ? Elle ne vaut que ce que le budget du jour permet
    // encore : le budget quotidien gagne TOUJOURS contre la durée de session.
    if let Some(grant) = active_grant(pool, child.id).await? {
        let grant_left = (grant.minutes * 60 - grant.consumed_secs).max(0);
        let remaining = grant_left.min(budget_left);
        if remaining > 0 {
            return Ok(GateDecision::Granted {
                remaining_secs: remaining,
            });
        }
        // Épuisée (par sa durée ou par le budget) : on la referme.
        close_grant(pool, grant.id).await?;
    }

    if budget_left <= 0 {
        return Ok(GateDecision::Blocked {
            reason: BlockReason::DailyBudgetSpent {
                used_min: used_secs / 60,
                budget_min: budget_secs / 60,
            },
        });
    }

    Ok(GateDecision::ExamAvailable {
        questions: child.questions_per_test,
        threshold_pct: child.pass_threshold_pct,
        max_grant_minutes: grantable_minutes(child, budget_left),
    })
}

/// Ce qu'un contrôle réussi peut rapporter : la durée de session, rabotée par
/// ce qui reste du budget. Au moins 1 minute tant qu'il reste des secondes,
/// pour ne pas offrir un contrôle qui ne rapporte rien.
fn grantable_minutes(child: &Child, budget_left_secs: i64) -> i64 {
    child.session_minutes.min(budget_left_secs / 60).max(1)
}

// ===== Concessions ==========================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Grant {
    pub id: i64,
    pub minutes: i64,
    pub consumed_secs: i64,
}

pub async fn active_grant(pool: &SqlitePool, child_id: i64) -> Result<Option<Grant>> {
    Ok(sqlx::query_as(
        "SELECT id, minutes, consumed_secs
         FROM grants
         WHERE child_id = ? AND closed_at IS NULL
         ORDER BY id DESC LIMIT 1",
    )
    .bind(child_id)
    .fetch_optional(pool)
    .await?)
}

/// Échange un contrôle réussi (ou rien du tout, si c'est le parent) contre du
/// temps. La durée est décidée ici par [`evaluate`], jamais par l'appelant :
/// c'est ce qui empêche de réclamer 30 minutes quand il n'en reste que 12.
pub async fn open_grant(
    pool: &SqlitePool,
    child: &Child,
    attempt_id: Option<i64>,
    minutes_override: Option<i64>,
) -> Result<i64> {
    let minutes = match minutes_override {
        Some(m) => m.max(1),
        None => {
            let budget_left =
                (child.budget_secs_today() - consumed_today(pool, child.id).await?).max(0);
            grantable_minutes(child, budget_left)
        }
    };

    // Une seule concession vivante à la fois.
    close_open_grants(pool, child.id).await?;

    let now = chrono::Utc::now().timestamp();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO grants (child_id, attempt_id, granted_at, minutes)
         VALUES (?, ?, ?, ?)
         RETURNING id",
    )
    .bind(child.id)
    .bind(attempt_id)
    .bind(now)
    .bind(minutes)
    .fetch_one(pool)
    .await?;

    tracing::info!(child = %child.name, %minutes, ?attempt_id, "concession ouverte");
    Ok(row.0)
}

pub async fn close_grant(pool: &SqlitePool, grant_id: i64) -> Result<()> {
    sqlx::query("UPDATE grants SET closed_at = ? WHERE id = ? AND closed_at IS NULL")
        .bind(chrono::Utc::now().timestamp())
        .bind(grant_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn close_open_grants(pool: &SqlitePool, child_id: i64) -> Result<()> {
    sqlx::query("UPDATE grants SET closed_at = ? WHERE child_id = ? AND closed_at IS NULL")
        .bind(chrono::Utc::now().timestamp())
        .bind(child_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ===== Consommation =========================================================

/// Secondes déjà consommées aujourd'hui par cet enfant.
pub async fn consumed_today(pool: &SqlitePool, child_id: i64) -> Result<i64> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT consumed_secs FROM daily_usage WHERE child_id = ? AND day = ?")
            .bind(child_id)
            .bind(today())
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(v,)| v).unwrap_or(0))
}

/// Appelé par le minuteur : `secs` est un intervalle mesuré de façon MONOTONE
/// côté client. On l'impute au grand livre du jour ET à la concession vivante.
///
/// `secs` est plafonné : un client (ou un bug) ne doit pas pouvoir brûler la
/// journée d'un coup, ni la créditer avec une valeur négative.
pub async fn consume(pool: &SqlitePool, child_id: i64, secs: i64) -> Result<()> {
    let secs = secs.clamp(0, 300);
    if secs == 0 {
        return Ok(());
    }

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO daily_usage (child_id, day, consumed_secs)
         VALUES (?, ?, ?)
         ON CONFLICT(child_id, day) DO UPDATE SET consumed_secs = consumed_secs + excluded.consumed_secs",
    )
    .bind(child_id)
    .bind(today())
    .bind(secs)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE grants SET consumed_secs = consumed_secs + ?
         WHERE child_id = ? AND closed_at IS NULL",
    )
    .bind(secs)
    .bind(child_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Date locale, pas UTC : « aujourd'hui » est celui de l'enfant, pas celui de
/// Greenwich.
fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}
