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
use chrono::{Datelike, Local, Timelike, Weekday};
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
    /// Hors des plages horaires autorisées. `next_window` est déjà en
    /// français (« demain à 17h00 ») : le motif EST l'écran.
    OutsideSchedule { next_window: Option<String> },
    /// Contrôle raté il y a moins de `exam_cooldown_minutes` : petite pause
    /// avant de réessayer, pour réfléchir au lieu de mitrailler.
    Cooldown { remaining_min: i64 },
}

// ===== Évaluation ===========================================================

pub async fn evaluate(pool: &SqlitePool, child: &Child) -> Result<GateDecision> {
    let sched = schedule_status(pool, child.id).await?;
    let budget_secs = child.budget_secs_today();
    let used_secs = consumed_today(pool, child.id).await?;
    let budget_left = (budget_secs - used_secs).max(0);

    // Une concession vivante ne vaut que ce que le budget du jour ET la plage
    // horaire permettent encore : le couvre-feu coupe une session en cours.
    if let Some(grant) = active_grant(pool, child.id).await? {
        let grant_left = (grant.minutes * 60 - grant.consumed_secs).max(0);
        // Une concession du parent (bypass) ne connaît ni budget ni couvre-feu :
        // le parent est devant l'écran, sa décision prime sur les règles.
        let remaining = if grant.bypass == 1 {
            grant_left
        } else {
            grant_left.min(budget_left).min(sched.window_left_secs)
        };
        if remaining > 0 {
            return Ok(GateDecision::Granted {
                remaining_secs: remaining,
            });
        }
        // Épuisée (durée, budget ou couvre-feu) : on la referme.
        close_grant(pool, grant.id).await?;
    }

    if !sched.in_window {
        return Ok(GateDecision::Blocked {
            reason: BlockReason::OutsideSchedule {
                next_window: sched.next_window,
            },
        });
    }

    if budget_left <= 0 {
        return Ok(GateDecision::Blocked {
            reason: BlockReason::DailyBudgetSpent {
                used_min: used_secs / 60,
                budget_min: budget_secs / 60,
            },
        });
    }

    // Contrôle raté récemment ? Pause obligatoire avant de réessayer. Seuls
    // les échecs comptent : pénaliser une réussite tuerait le renouvellement.
    if child.exam_cooldown_minutes > 0 {
        if let Some(remaining_min) = cooldown_remaining(pool, child).await? {
            return Ok(GateDecision::Blocked {
                reason: BlockReason::Cooldown { remaining_min },
            });
        }
    }

    Ok(GateDecision::ExamAvailable {
        questions: child.questions_per_test,
        threshold_pct: child.pass_threshold_pct,
        max_grant_minutes: grantable_minutes(
            child,
            budget_left.min(sched.window_left_secs),
        ),
    })
}

/// Ce qu'un contrôle réussi peut rapporter : la durée de session, rabotée par
/// ce qui reste du budget et de la fenêtre horaire. Au moins 1 minute tant
/// qu'il reste des secondes, pour ne pas offrir un contrôle qui ne rapporte
/// rien.
fn grantable_minutes(child: &Child, left_secs: i64) -> i64 {
    child.session_minutes.min(left_secs / 60).max(1)
}

/// Minutes restantes de pause après le dernier contrôle RATÉ, ou `None` si la
/// pause est finie (ou qu'il n'y a jamais eu d'échec).
async fn cooldown_remaining(pool: &SqlitePool, child: &Child) -> Result<Option<i64>> {
    let last_fail: Option<(i64,)> = sqlx::query_as(
        "SELECT finished_at FROM attempts
         WHERE child_id = ? AND passed = 0 AND finished_at IS NOT NULL
         ORDER BY finished_at DESC LIMIT 1",
    )
    .bind(child.id)
    .fetch_optional(pool)
    .await?;

    let Some((failed_at,)) = last_fail else {
        return Ok(None);
    };
    let until = failed_at + child.exam_cooldown_minutes * 60;
    let now = chrono::Utc::now().timestamp();
    if now >= until {
        return Ok(None);
    }
    // Arrondi vers le haut : « 1 minute » tant qu'il reste des secondes.
    Ok(Some((until - now + 59) / 60))
}

// ===== Plages horaires ======================================================

/// Position de l'enfant par rapport à ses plages horaires.
struct ScheduleStatus {
    /// Vrai si aucune plage n'est définie (liberté totale) ou si on est dans
    /// une fenêtre.
    in_window: bool,
    /// Secondes avant la fin de la fenêtre courante. `i64::MAX` sans plages.
    window_left_secs: i64,
    /// Prochaine ouverture, déjà humanisée (« demain à 17h00 »).
    next_window: Option<String>,
}

const WEEKDAYS_FR: [&str; 7] = [
    "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
];

async fn schedule_status(pool: &SqlitePool, child_id: i64) -> Result<ScheduleStatus> {
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT weekday, start_min, end_min FROM schedules WHERE child_id = ?",
    )
    .bind(child_id)
    .fetch_all(pool)
    .await?;

    // Aucune plage définie = aucune restriction. Le contrôle horaire est
    // opt-in, enfant par enfant.
    if rows.is_empty() {
        return Ok(ScheduleStatus {
            in_window: true,
            window_left_secs: i64::MAX,
            next_window: None,
        });
    }

    let now = Local::now();
    let today_wd = now.weekday().num_days_from_monday() as i64; // 0 = lundi
    let now_min = (now.hour() * 60 + now.minute()) as i64;

    // Dans une fenêtre ? En cas de chevauchement, la fin la plus tardive gagne.
    let mut window_end: Option<i64> = None;
    for &(wd, start, end) in &rows {
        if wd == today_wd && start <= now_min && now_min < end {
            window_end = Some(window_end.map_or(end, |e: i64| e.max(end)));
        }
    }
    if let Some(end) = window_end {
        return Ok(ScheduleStatus {
            in_window: true,
            window_left_secs: (end - now_min) * 60,
            next_window: None,
        });
    }

    // Hors fenêtre : chercher la prochaine ouverture sur 7 jours.
    let mut next: Option<String> = None;
    'search: for off in 0..=7i64 {
        let wd = (today_wd + off) % 7;
        let mut starts: Vec<i64> = rows
            .iter()
            .filter(|&&(d, start, _)| d == wd && (off > 0 || start > now_min))
            .map(|&(_, start, _)| start)
            .collect();
        starts.sort_unstable();
        if let Some(start) = starts.first() {
            let time = format!("{:02}h{:02}", start / 60, start % 60);
            next = Some(match off {
                0 => format!("aujourd'hui à {time}"),
                1 => format!("demain à {time}"),
                _ => format!("{} à {time}", WEEKDAYS_FR[wd as usize]),
            });
            break 'search;
        }
    }

    Ok(ScheduleStatus {
        in_window: false,
        window_left_secs: 0,
        next_window: next,
    })
}

// ===== Concessions ==========================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Grant {
    pub id: i64,
    pub minutes: i64,
    pub consumed_secs: i64,
    /// 1 = concession du parent : ignore budget et plages horaires.
    pub bypass: i64,
}

pub async fn active_grant(pool: &SqlitePool, child_id: i64) -> Result<Option<Grant>> {
    Ok(sqlx::query_as(
        "SELECT id, minutes, consumed_secs, bypass
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
    bypass: bool,
) -> Result<i64> {
    let minutes = match minutes_override {
        Some(m) => m.max(1),
        None => {
            let sched = schedule_status(pool, child.id).await?;
            let budget_left =
                (child.budget_secs_today() - consumed_today(pool, child.id).await?).max(0);
            grantable_minutes(child, budget_left.min(sched.window_left_secs))
        }
    };

    // Une seule concession vivante à la fois.
    close_open_grants(pool, child.id).await?;

    let now = chrono::Utc::now().timestamp();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO grants (child_id, attempt_id, granted_at, minutes, bypass)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(child.id)
    .bind(attempt_id)
    .bind(now)
    .bind(minutes)
    .bind(if bypass { 1 } else { 0 })
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
