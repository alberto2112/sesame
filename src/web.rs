use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use rust_embed::RustEmbed;
use sqlx::SqlitePool;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tower_cookies::CookieManagerLayer;
use tower_http::trace::TraceLayer;

use crate::admin;
use crate::config::Config;
use crate::quiz::{self, GradedAttempt, QuizQuestion, Submission};

pub struct GameSession {
    pub child: Child,
    pub started_at: i64,
    pub kill_at: i64,
}

pub type GameSlot = Arc<Mutex<Option<GameSession>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub cfg: Arc<Config>,
    pub game: GameSlot,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/submit", post(submit))
        .route("/game", get(game))
        .route("/game/start", post(game_start))
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
    questions: Vec<QuizQuestion>,
    threshold_fmt: String,
    ids_csv: String,
}

#[derive(Template)]
#[template(path = "result.html")]
struct ResultTemplate {
    attempt: GradedAttempt,
    score_fmt: String,
    threshold_fmt: String,
}

#[derive(Template)]
#[template(path = "game.html")]
struct GameTemplate {
    kill_minutes: i64,
    refresh_seconds: i64,
    elapsed_minutes: i64,
}

// ===== Handlers =====

async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let n = read_setting_usize(&state.pool, "questions_per_test", 10).await?;
    let threshold = read_setting_f64(&state.pool, "pass_threshold_pct", 70.0).await?;

    let questions = quiz::pick_questions(&state.pool, n).await?;
    let ids_csv = questions
        .iter()
        .map(|q| q.id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let tmpl = QuizTemplate {
        questions,
        threshold_fmt: format!("{threshold:.0}"),
        ids_csv,
    };
    Ok(render(tmpl))
}

async fn submit(
    State(state): State<AppState>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let (question_ids, mut chosen) = parse_form(&pairs)?;
    for qid in &question_ids {
        chosen.entry(*qid).or_default();
    }

    let attempt = quiz::grade(&state.pool, &chosen).await?;
    persist_attempt(&state.pool, &attempt).await?;

    let tmpl = ResultTemplate {
        score_fmt: format!("{:.0}", attempt.score_pct),
        threshold_fmt: format!("{:.0}", attempt.threshold_pct),
        attempt,
    };
    Ok(render(tmpl))
}

async fn game(State(state): State<AppState>) -> Result<Response, AppError> {
    let kill_minutes = read_setting_i64(&state.pool, "kill_interval_minutes", 30).await?;
    let guard = state.game.lock().await;
    let Some(session) = guard.as_ref() else {
        return Ok(Redirect::to("/").into_response());
    };
    let now = chrono::Utc::now().timestamp();
    let remaining = (session.kill_at - now).max(0);
    let refresh_seconds = remaining + 2;
    let elapsed_minutes = (now - session.started_at).max(0) / 60;
    drop(guard);
    Ok(render(GameTemplate {
        kill_minutes,
        refresh_seconds,
        elapsed_minutes,
    }))
}

async fn game_start(State(state): State<AppState>) -> Result<Response, AppError> {
    let kill_minutes = read_setting_i64(&state.pool, "kill_interval_minutes", 30).await?;
    let mut guard = state.game.lock().await;
    if guard.is_some() {
        return Ok(Redirect::to("/game").into_response());
    }

    let binary = &state.cfg.paths.game_binary;
    let child = Command::new(binary)
        .spawn()
        .with_context(|| format!("spawning game binary {}", binary.display()))?;
    let started_at = chrono::Utc::now().timestamp();
    let kill_at = started_at + kill_minutes * 60;
    *guard = Some(GameSession {
        child,
        started_at,
        kill_at,
    });
    drop(guard);

    tracing::info!(%kill_minutes, %kill_at, "game session started");
    spawn_watchdog(state.game.clone(), kill_at);

    Ok(Redirect::to("/game").into_response())
}

fn spawn_watchdog(slot: GameSlot, kill_at: i64) {
    tokio::spawn(async move {
        let now = chrono::Utc::now().timestamp();
        let wait = (kill_at - now).max(0) as u64;
        tokio::time::sleep(Duration::from_secs(wait)).await;

        let mut guard = slot.lock().await;
        // Sanity check: another session might have replaced this one.
        let take = matches!(guard.as_ref(), Some(s) if s.kill_at == kill_at);
        if !take {
            return;
        }
        if let Some(mut session) = guard.take() {
            match session.child.kill().await {
                Ok(()) => tracing::info!(%kill_at, "game session killed by watchdog"),
                Err(err) => tracing::warn!(?err, "failed to kill game child"),
            }
        }
    });
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

async fn persist_attempt(pool: &SqlitePool, attempt: &GradedAttempt) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO attempts (started_at, finished_at, score_pct, passed)
         VALUES (?, ?, ?, ?)
         RETURNING id",
    )
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

// ===== Settings helpers =====

async fn read_setting_str(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

async fn read_setting_usize(pool: &SqlitePool, key: &str, default: usize) -> Result<usize> {
    Ok(read_setting_str(pool, key)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(default))
}

async fn read_setting_i64(pool: &SqlitePool, key: &str, default: i64) -> Result<i64> {
    Ok(read_setting_str(pool, key)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(default))
}

async fn read_setting_f64(pool: &SqlitePool, key: &str, default: f64) -> Result<f64> {
    Ok(read_setting_str(pool, key)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(default))
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
        (
            self.status,
            format!("Erreur : {}", self.inner),
        )
            .into_response()
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
