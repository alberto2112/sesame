use std::collections::HashMap;

use anyhow::{Context, Result};
use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::{get, post};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower_cookies::{Cookie, CookieManagerLayer, Cookies};
use tower_http::trace::TraceLayer;

use crate::admin;
use crate::policy::{self, BlockReason, Child, GateDecision};
use crate::quiz::{self, GradedAttempt, QuizQuestion, Submission};

/// Cookie qui retient quel enfant est devant l'écran, posé par le sélecteur de
/// profils (/profiles). Cookie de session : à chaque redémarrage du navigateur
/// (donc du kiosque), on redemande qui est là.
pub const CHILD_COOKIE_NAME: &str = "sesame_child";

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/profiles", get(profiles_get))
        .route("/profile", post(profile_post))
        .route("/submit", post(submit))
        .route("/unlock", post(unlock))
        .route("/api/status", get(api_status))
        .route("/api/heartbeat", post(api_heartbeat))
        .nest("/admin", admin::router())
        .route("/static/*path", get(static_asset))
        .layer(CookieManagerLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ===== Static assets (embedded in the binary) =====

#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticAssets;

async fn static_asset(Path(path): Path<String>) -> Response {
    match StaticAssets::get(&path) {
        Some(asset) => {
            let mime = asset.metadata.mimetype().to_string();
            ([(header::CONTENT_TYPE, mime)], asset.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ===== Templates =====

#[derive(Template)]
#[template(path = "quiz.html")]
struct QuizTemplate {
    child_name: String,
    child_avatar: String,
    questions: Vec<QuizQuestion>,
    threshold_fmt: String,
    grant_minutes: i64,
    ids_csv: String,
}

#[derive(Template)]
#[template(path = "result.html")]
struct ResultTemplate {
    attempt: GradedAttempt,
    attempt_id: i64,
    score_fmt: String,
    threshold_fmt: String,
    grant_minutes: i64,
}

#[derive(Template)]
#[template(path = "granted.html")]
struct GrantedTemplate {
    child_name: String,
    child_avatar: String,
    remaining_minutes: i64,
}

#[derive(Template)]
#[template(path = "blocked.html")]
struct BlockedTemplate {
    child_name: String,
    child_avatar: String,
    emoji: String,
    title: String,
    detail: String,
}

#[derive(Template)]
#[template(path = "profiles.html")]
struct ProfilesTemplate {
    children: Vec<Child>,
}

// ===== Handlers =====

/// Point d'entrée unique de l'enfant. Ce qu'il voit est décidé par le moteur de
/// politiques, jamais par ce handler.
async fn index(State(state): State<AppState>, cookies: Cookies) -> Result<Response, AppError> {
    let Some(child) = resolve_child(&state, &cookies).await? else {
        return Ok(Redirect::to("/profiles").into_response());
    };

    match policy::evaluate(&state.pool, &child).await? {
        GateDecision::Granted { remaining_secs } => Ok(render(GrantedTemplate {
            child_name: child.name,
            child_avatar: child.avatar,
            remaining_minutes: (remaining_secs + 59) / 60,
        })),

        GateDecision::Blocked { reason } => Ok(render(blocked_page(&child, &reason))),

        GateDecision::ExamAvailable {
            questions,
            threshold_pct,
            max_grant_minutes,
        } => {
            let questions = quiz::pick_questions(
                &state.pool,
                questions as usize,
                child.difficulty_min,
                child.difficulty_max,
            )
            .await?;

            // Aucune question dans sa plage de difficulté : un formulaire vide
            // serait un cul-de-sac pour l'enfant. On explique, et l'admin voit
            // le même problème signalé sur /admin/children.
            if questions.is_empty() {
                return Ok(render(BlockedTemplate {
                    child_name: child.name,
                    child_avatar: child.avatar,
                    emoji: "🧐".to_string(),
                    title: "Pas encore de questions pour toi".to_string(),
                    detail: "Aucune question ne correspond à ton niveau. \
                             Demande à un parent d'en ajouter !"
                        .to_string(),
                }));
            }

            let ids_csv = questions
                .iter()
                .map(|q| q.id.to_string())
                .collect::<Vec<_>>()
                .join(",");

            Ok(render(QuizTemplate {
                child_name: child.name,
                child_avatar: child.avatar,
                questions,
                threshold_fmt: format!("{threshold_pct:.0}"),
                grant_minutes: max_grant_minutes,
                ids_csv,
            }))
        }
    }
}

/// Corrige le contrôle et affiche la correction. **N'ouvre PAS la concession** :
/// le chrono ne doit pas tourner pendant que l'enfant lit ses erreurs. C'est le
/// bouton de la page de résultat (`POST /unlock`) qui démarre le temps.
async fn submit(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let Some(child) = resolve_child(&state, &cookies).await? else {
        return Ok(Redirect::to("/profiles").into_response());
    };

    let (question_ids, mut chosen) = parse_form(&pairs)?;
    for qid in &question_ids {
        chosen.entry(*qid).or_default();
    }

    let attempt = quiz::grade(&state.pool, &chosen, child.pass_threshold_pct).await?;
    let attempt_id = persist_attempt(&state.pool, child.id, &attempt).await?;

    // Ce que ce contrôle rapportera s'il est réussi — calculé maintenant, à
    // titre indicatif : /unlock le recalculera, car le budget a pu bouger.
    let grant_minutes = match policy::evaluate(&state.pool, &child).await? {
        GateDecision::ExamAvailable {
            max_grant_minutes, ..
        } => max_grant_minutes,
        _ => 0,
    };

    Ok(render(ResultTemplate {
        score_fmt: format!("{:.0}", attempt.score_pct),
        threshold_fmt: format!("{:.0}", attempt.threshold_pct),
        attempt_id,
        grant_minutes,
        attempt,
    }))
}

#[derive(Deserialize)]
struct UnlockForm {
    attempt_id: i64,
}

/// Échange un contrôle réussi contre du temps. Tout est revérifié ici — c'est
/// la seule porte par laquelle du temps peut entrer.
async fn unlock(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(form): Form<UnlockForm>,
) -> Result<Response, AppError> {
    let Some(child) = resolve_child(&state, &cookies).await? else {
        return Ok(Redirect::to("/profiles").into_response());
    };

    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT passed, COALESCE(child_id, 0) FROM attempts WHERE id = ?")
            .bind(form.attempt_id)
            .fetch_optional(&state.pool)
            .await?;

    let Some((passed, attempt_child)) = row else {
        return Err(AppError::bad_request("contrôle introuvable"));
    };
    if passed != 1 {
        return Err(AppError::bad_request("ce contrôle n'a pas été réussi"));
    }
    if attempt_child != child.id {
        return Err(AppError::bad_request("ce contrôle n'est pas le tien"));
    }

    // Un contrôle réussi ne s'échange qu'UNE fois : renvoyer le formulaire ne
    // donne pas 30 minutes de plus. L'index unique sur grants(attempt_id)
    // garantit la même chose au niveau de la base.
    let already: Option<(i64,)> = sqlx::query_as("SELECT id FROM grants WHERE attempt_id = ?")
        .bind(form.attempt_id)
        .fetch_optional(&state.pool)
        .await?;
    if already.is_some() {
        return Ok(Redirect::to("/").into_response());
    }

    // La durée est décidée par le moteur, jamais par le formulaire.
    if let GateDecision::ExamAvailable { .. } = policy::evaluate(&state.pool, &child).await? {
        policy::open_grant(&state.pool, &child, Some(form.attempt_id), None).await?;
    }

    Ok(Redirect::to("/").into_response())
}

// ===== API (kiosque, fenêtre de verrouillage, minuteur) =====

#[derive(Serialize)]
struct StatusResponse {
    child_id: i64,
    child_name: String,
    #[serde(flatten)]
    decision: GateDecision,
}

#[derive(Deserialize)]
struct StatusQuery {
    /// Le kiosque et le minuteur n'ont pas de cookies : ils demandent le
    /// statut d'un enfant précis. Sans ce paramètre, on retombe sur le cookie.
    child_id: Option<i64>,
}

async fn api_status(
    State(state): State<AppState>,
    cookies: Cookies,
    Query(q): Query<StatusQuery>,
) -> Result<Response, AppError> {
    let child = match q.child_id {
        Some(id) => policy::load_child(&state.pool, id)
            .await?
            .ok_or_else(|| AppError::bad_request("enfant inconnu"))?,
        None => match resolve_child(&state, &cookies).await? {
            Some(c) => c,
            // Plusieurs enfants et aucun choisi : le client doit afficher le
            // sélecteur (ou passer ?child_id=).
            None => {
                return Ok(Json(serde_json::json!({ "state": "profile_required" }))
                    .into_response());
            }
        },
    };

    let decision = policy::evaluate(&state.pool, &child).await?;
    Ok(Json(StatusResponse {
        child_id: child.id,
        child_name: child.name,
        decision,
    })
    .into_response())
}

#[derive(Deserialize)]
struct HeartbeatBody {
    child_id: i64,
    /// Secondes écoulées depuis le dernier battement, mesurées de façon
    /// MONOTONE par le minuteur. Le serveur ne fait qu'additionner : l'horloge
    /// de la machine n'entre jamais dans le calcul.
    secs: i64,
}

async fn api_heartbeat(
    State(state): State<AppState>,
    Json(body): Json<HeartbeatBody>,
) -> Result<Json<GateDecision>, AppError> {
    let child = policy::load_child(&state.pool, body.child_id)
        .await?
        .ok_or_else(|| AppError::bad_request("enfant inconnu"))?;

    policy::consume(&state.pool, child.id, body.secs).await?;
    Ok(Json(policy::evaluate(&state.pool, &child).await?))
}

// ===== Sélecteur de profils =====

async fn profiles_get(State(state): State<AppState>) -> Result<Response, AppError> {
    let children = policy::list_children(&state.pool).await?;
    if children.is_empty() {
        return Err(AppError::bad_request(
            "aucun enfant configuré — le parent doit en créer un depuis /admin/children",
        ));
    }
    Ok(render(ProfilesTemplate { children }))
}

#[derive(Deserialize)]
struct ProfileForm {
    child_id: i64,
}

async fn profile_post(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(form): Form<ProfileForm>,
) -> Result<Response, AppError> {
    // On ne pose le cookie que pour un enfant réel et actif : impossible de
    // forger un profil inexistant en éditant la requête.
    let child = policy::load_child(&state.pool, form.child_id)
        .await?
        .ok_or_else(|| AppError::bad_request("enfant inconnu"))?;

    let mut c = Cookie::new(CHILD_COOKIE_NAME, child.id.to_string());
    c.set_path("/");
    c.set_http_only(true);
    cookies.add(c);

    Ok(Redirect::to("/").into_response())
}

// ===== Enfant courant =====

/// Qui est devant l'écran ?
/// - cookie valide → cet enfant ;
/// - pas de cookie et UN SEUL enfant actif → lui (pas de sélecteur inutile
///   dans une famille à enfant unique) ;
/// - pas de cookie et plusieurs enfants → `None`, l'appelant envoie au
///   sélecteur ;
/// - aucun enfant actif → erreur (il faut passer par /admin).
async fn resolve_child(state: &AppState, cookies: &Cookies) -> Result<Option<Child>, AppError> {
    if let Some(c) = cookies.get(CHILD_COOKIE_NAME) {
        if let Ok(id) = c.value().parse::<i64>() {
            if let Some(child) = policy::load_child(&state.pool, id).await? {
                return Ok(Some(child));
            }
        }
    }

    let mut children = policy::list_children(&state.pool).await?;
    match children.len() {
        0 => Err(AppError::bad_request(
            "aucun enfant configuré — le parent doit en créer un depuis /admin/children",
        )),
        1 => Ok(Some(children.remove(0))),
        _ => Ok(None),
    }
}

// ===== Écran de blocage =====

fn blocked_page(child: &Child, reason: &BlockReason) -> BlockedTemplate {
    let (emoji, title, detail) = match reason {
        BlockReason::DailyBudgetSpent {
            used_min,
            budget_min,
        } => (
            "⏳",
            "C'est fini pour aujourd'hui".to_string(),
            format!(
                "Tu as utilisé tes {budget_min} minutes ({used_min} min au compteur). \
                 Un contrôle de plus n'y changera rien — à demain !"
            ),
        ),

        BlockReason::OutsideSchedule { next_window } => (
            "🌙",
            "C'est l'heure de dormir".to_string(),
            match next_window {
                Some(next) => format!("Ce n'est pas le moment d'utiliser l'ordinateur. Tu pourras revenir {next}."),
                None => "Ce n'est pas le moment d'utiliser l'ordinateur.".to_string(),
            },
        ),

        BlockReason::Cooldown { remaining_min } => (
            "🧘",
            "Petite pause !".to_string(),
            format!(
                "Ton dernier contrôle n'est pas passé. Respire, relis tes leçons… \
                 tu pourras réessayer dans {remaining_min} minute{}.",
                if *remaining_min > 1 { "s" } else { "" }
            ),
        ),
    };

    BlockedTemplate {
        child_name: child.name.clone(),
        child_avatar: child.avatar.clone(),
        emoji: emoji.to_string(),
        title,
        detail,
    }
}

// ===== Form parsing =====

fn parse_form(pairs: &[(String, String)]) -> Result<(Vec<i64>, Submission), AppError> {
    let mut question_ids: Vec<i64> = Vec::new();
    let mut chosen: HashMap<i64, Vec<i64>> = HashMap::new();

    for (k, v) in pairs {
        if k == "question_ids" {
            for piece in v.split(',') {
                let id: i64 = piece
                    .trim()
                    .parse()
                    .with_context(|| format!("invalid question_id '{piece}'"))?;
                question_ids.push(id);
            }
        } else if let Some(suffix) = k.strip_prefix("q_") {
            let qid: i64 = suffix
                .parse()
                .with_context(|| format!("invalid q_ key '{suffix}'"))?;
            let aid: i64 = v
                .parse()
                .with_context(|| format!("invalid answer id '{v}'"))?;
            chosen.entry(qid).or_default().push(aid);
        }
    }

    if question_ids.is_empty() {
        return Err(AppError::bad_request("aucune question soumise"));
    }
    Ok((question_ids, chosen))
}

// ===== Persistence =====

async fn persist_attempt(pool: &SqlitePool, child_id: i64, attempt: &GradedAttempt) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO attempts (child_id, started_at, finished_at, score_pct, passed)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(child_id)
    .bind(now)
    .bind(now)
    .bind(attempt.score_pct)
    .bind(if attempt.passed { 1 } else { 0 })
    .fetch_one(&mut *tx)
    .await?;
    let attempt_id = row.0;

    for q in &attempt.questions {
        sqlx::query(
            "INSERT INTO attempt_questions (attempt_id, question_id, answered_correctly)
             VALUES (?, ?, ?)",
        )
        .bind(attempt_id)
        .bind(q.question_id)
        .bind(if q.correct { 1 } else { 0 })
        .execute(&mut *tx)
        .await?;

        for a in &q.answers {
            sqlx::query(
                "INSERT INTO attempt_answers
                   (attempt_id, question_id, kind_snapshot, statement_snapshot,
                    answer_id, answer_text_snapshot, was_chosen, is_correct)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(attempt_id)
            .bind(q.question_id)
            .bind(&q.kind)
            .bind(&q.statement)
            .bind(a.answer_id)
            .bind(&a.text)
            .bind(if a.was_chosen { 1 } else { 0 })
            .bind(if a.is_correct { 1 } else { 0 })
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(attempt_id)
}

// ===== Response helper =====

pub fn render<T: Template>(tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => axum::response::Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "template render failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("erreur de rendu : {err}"),
            )
                .into_response()
        }
    }
}

// ===== Error type =====

pub struct AppError {
    pub status: StatusCode,
    pub inner: anyhow::Error,
}

impl AppError {
    pub fn bad_request<M: Into<String>>(msg: M) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            inner: anyhow::anyhow!(msg.into()),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.inner, "request failed");
        (self.status, format!("Erreur : {}", self.inner)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            inner: e.into(),
        }
    }
}
