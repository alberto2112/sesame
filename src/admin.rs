use std::collections::HashMap;

use anyhow::Context;
use askama::Template;
use axum::Router;
use axum::extract::{Form, Multipart, Path as AxPath, Query, State};
use axum::http::{StatusCode, request::Parts};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use tower_cookies::{Cookie, Cookies};

use crate::auth;
use crate::importer::{self, ImportFile};
use crate::web::{AppError, AppState, render};

// ===== Router ================================================================

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/setup", get(setup_get).post(setup_post))
        .route("/login", get(login_get).post(login_post))
        .route("/logout", post(logout_post))
        .route("/questions", get(questions_list))
        .route("/questions/new", get(question_new_get).post(question_new_post))
        .route("/questions/:id/edit", get(question_edit_get).post(question_edit_post))
        .route("/questions/:id/delete", post(question_delete_post))
        .route("/subjects", get(subjects_get).post(subjects_post))
        .route("/subjects/:id/delete", post(subject_delete_post))
        .route("/settings", get(settings_get).post(settings_post))
        .route("/import", get(import_get).post(import_post))
        .route("/history", get(history_list))
        .route("/history/:id", get(history_detail))
}

// ===== Auth extractor ========================================================

pub struct AdminAuth;

#[axum::async_trait]
impl axum::extract::FromRequestParts<AppState> for AdminAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookies = Cookies::from_request_parts(parts, state)
            .await
            .map_err(|_| Redirect::to("/admin/login").into_response())?;

        let token = match cookies.get(auth::SESSION_COOKIE_NAME) {
            Some(c) => c.value().to_string(),
            None => return Err(redirect_login_or_setup(state).await),
        };

        match auth::validate_session(&state.pool, &token).await {
            Ok(true) => Ok(AdminAuth),
            _ => Err(redirect_login_or_setup(state).await),
        }
    }
}

async fn redirect_login_or_setup(state: &AppState) -> Response {
    match auth::password_is_set(&state.pool).await {
        Ok(true) => Redirect::to("/admin/login").into_response(),
        Ok(false) => Redirect::to("/admin/setup").into_response(),
        Err(e) => {
            tracing::error!(?e, "checking password setup");
            (StatusCode::INTERNAL_SERVER_ERROR, "erreur interne").into_response()
        }
    }
}

// ===== Templates =============================================================

#[derive(Template)]
#[template(path = "admin/setup.html")]
struct SetupTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
struct DashboardTemplate {
    nb_questions: i64,
    nb_subjects: i64,
    nb_attempts: i64,
}

#[derive(Template)]
#[template(path = "admin/questions_list.html")]
struct QuestionsListTemplate {
    questions: Vec<QuestionRow>,
    subjects: Vec<SubjectOption>,
    flash: Option<String>,
}

struct SubjectOption {
    id: i64,
    name: String,
    selected: bool,
}

struct QuestionRow {
    id: i64,
    statement: String,
    kind: String,
    subject_name: String,
    nb_answers: i64,
}

#[derive(Template)]
#[template(path = "admin/question_form.html")]
struct QuestionFormTemplate {
    title: String,
    action: String,
    subjects: Vec<SubjectOption>,
    statement: String,
    explanation: String,
    kind: String,
    answers: Vec<(String, bool)>,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/subjects.html")]
struct SubjectsTemplate {
    // (id, nom, poids, nb_questions, activée)
    subjects: Vec<(i64, String, f64, i64, bool)>,
    flash: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/settings.html")]
struct SettingsTemplate {
    questions_per_test: String,
    pass_threshold_pct: String,
    kill_interval_minutes: String,
    flash: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/import.html")]
struct ImportTemplate {
    flash: Option<String>,
    report: Option<ImportReportView>,
}

struct ImportReportView {
    subjects_created: usize,
    subjects_skipped: usize,
    questions_imported: usize,
    questions_failed: Vec<(usize, String)>,
}

#[derive(Template)]
#[template(path = "admin/history_list.html")]
struct HistoryListTemplate {
    attempts: Vec<HistoryAttemptRow>,
}

struct HistoryAttemptRow {
    id: i64,
    when: String,
    score_fmt: String,
    passed: bool,
}

#[derive(Template)]
#[template(path = "admin/history_detail.html")]
struct HistoryDetailTemplate {
    attempt_id: i64,
    when: String,
    score_fmt: String,
    passed: bool,
    questions: Vec<HistoryDetailQuestion>,
}

struct HistoryDetailQuestion {
    statement: String,
    kind: String,
    correct_overall: bool,
    answers: Vec<HistoryDetailAnswer>,
}

struct HistoryDetailAnswer {
    text: String,
    is_correct: bool,
    was_chosen: bool,
}

// ===== Auth handlers =========================================================

async fn setup_get(State(state): State<AppState>) -> Result<Response, AppError> {
    if auth::password_is_set(&state.pool).await? {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    Ok(render(SetupTemplate { error: None }))
}

#[derive(Deserialize)]
struct SetupForm {
    password: String,
    confirm: String,
}

async fn setup_post(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(form): Form<SetupForm>,
) -> Result<Response, AppError> {
    if auth::password_is_set(&state.pool).await? {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    if form.password.len() < 6 {
        return Ok(render(SetupTemplate {
            error: Some("Le mot de passe doit faire au moins 6 caractères.".into()),
        }));
    }
    if form.password != form.confirm {
        return Ok(render(SetupTemplate {
            error: Some("Les deux mots de passe ne correspondent pas.".into()),
        }));
    }
    auth::set_password(&state.pool, &form.password).await?;
    issue_session_cookie(&state, &cookies).await?;
    Ok(Redirect::to("/admin").into_response())
}

async fn login_get(State(state): State<AppState>) -> Result<Response, AppError> {
    if !auth::password_is_set(&state.pool).await? {
        return Ok(Redirect::to("/admin/setup").into_response());
    }
    Ok(render(LoginTemplate { error: None }))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn login_post(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let hash = match auth::read_password_hash(&state.pool).await? {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(Redirect::to("/admin/setup").into_response()),
    };
    if !auth::verify_password(&form.password, &hash) {
        return Ok(render(LoginTemplate {
            error: Some("Mot de passe incorrect.".into()),
        }));
    }
    issue_session_cookie(&state, &cookies).await?;
    Ok(Redirect::to("/admin").into_response())
}

async fn logout_post(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<Response, AppError> {
    if let Some(c) = cookies.get(auth::SESSION_COOKIE_NAME) {
        let token = c.value().to_string();
        let _ = auth::delete_session(&state.pool, &token).await;
        let mut clear = Cookie::new(auth::SESSION_COOKIE_NAME, "");
        clear.set_path("/");
        clear.set_max_age(tower_cookies::cookie::time::Duration::seconds(0));
        cookies.add(clear);
    }
    Ok(Redirect::to("/admin/login").into_response())
}

async fn issue_session_cookie(state: &AppState, cookies: &Cookies) -> Result<(), AppError> {
    let _ = auth::purge_expired_sessions(&state.pool).await;
    let token = auth::create_session(&state.pool).await?;
    let mut c = Cookie::new(auth::SESSION_COOKIE_NAME, token);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Lax);
    c.set_max_age(tower_cookies::cookie::time::Duration::days(30));
    cookies.add(c);
    Ok(())
}

// ===== Dashboard =============================================================

async fn dashboard(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let nb_questions: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM questions")
        .fetch_one(&state.pool)
        .await?;
    let nb_subjects: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM subjects")
        .fetch_one(&state.pool)
        .await?;
    let nb_attempts: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM attempts")
        .fetch_one(&state.pool)
        .await?;
    Ok(render(DashboardTemplate {
        nb_questions: nb_questions.0,
        nb_subjects: nb_subjects.0,
        nb_attempts: nb_attempts.0,
    }))
}

// ===== Questions: list =======================================================

#[derive(Deserialize)]
struct QuestionsListQuery {
    subject: Option<i64>,
    msg: Option<String>,
}

async fn questions_list(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<QuestionsListQuery>,
) -> Result<Response, AppError> {
    let subjects: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM subjects ORDER BY name")
            .fetch_all(&state.pool)
            .await?;

    let rows: Vec<(i64, String, String, String, i64)> = if let Some(sid) = q.subject {
        sqlx::query_as(
            "SELECT q.id, q.statement, q.kind, s.name, COUNT(a.id)
             FROM questions q
             JOIN subjects s ON s.id = q.subject_id
             LEFT JOIN answers a ON a.question_id = q.id
             WHERE q.subject_id = ?
             GROUP BY q.id ORDER BY q.id DESC",
        )
        .bind(sid)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT q.id, q.statement, q.kind, s.name, COUNT(a.id)
             FROM questions q
             JOIN subjects s ON s.id = q.subject_id
             LEFT JOIN answers a ON a.question_id = q.id
             GROUP BY q.id ORDER BY q.id DESC",
        )
        .fetch_all(&state.pool)
        .await?
    };

    let questions = rows
        .into_iter()
        .map(|(id, st, kd, sn, na)| QuestionRow {
            id,
            statement: st,
            kind: kd,
            subject_name: sn,
            nb_answers: na,
        })
        .collect();

    let subject_opts = subjects
        .into_iter()
        .map(|(id, name)| SubjectOption {
            id,
            selected: q.subject == Some(id),
            name,
        })
        .collect();

    Ok(render(QuestionsListTemplate {
        questions,
        subjects: subject_opts,
        flash: q.msg,
    }))
}

fn build_subject_options(rows: Vec<(i64, String)>, selected: Option<i64>) -> Vec<SubjectOption> {
    rows.into_iter()
        .map(|(id, name)| SubjectOption {
            id,
            selected: selected == Some(id),
            name,
        })
        .collect()
}

// ===== Questions: create =====================================================

async fn question_new_get(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let subjects = list_subjects(&state).await?;
    Ok(render(QuestionFormTemplate {
        title: "Nouvelle question".into(),
        action: "/admin/questions/new".into(),
        subjects: build_subject_options(subjects, None),
        statement: String::new(),
        explanation: String::new(),
        kind: "single".into(),
        answers: vec![(String::new(), false); 6],
        error: None,
    }))
}

async fn question_new_post(
    _: AdminAuth,
    State(state): State<AppState>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let parsed = parse_question_form(&pairs);
    let subjects = list_subjects(&state).await?;

    let parsed = match parsed {
        Ok(p) => p,
        Err(msg) => {
            let sel: Option<i64> = pair_get(&pairs, "subject_id").parse().ok();
            return Ok(render(QuestionFormTemplate {
                title: "Nouvelle question".into(),
                action: "/admin/questions/new".into(),
                subjects: build_subject_options(subjects, sel),
                statement: pair_get(&pairs, "statement"),
                explanation: pair_get(&pairs, "explanation"),
                kind: pair_get_or(&pairs, "kind", "single"),
                answers: collect_answer_pairs(&pairs),
                error: Some(msg),
            }));
        }
    };

    let now = chrono::Utc::now().timestamp();
    let mut tx = state.pool.begin().await?;
    let inserted: (i64,) = sqlx::query_as(
        "INSERT INTO questions (subject_id, kind, statement, explanation, created_at)
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(parsed.subject_id)
    .bind(&parsed.kind)
    .bind(&parsed.statement)
    .bind(parsed.explanation.as_ref())
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    for (text, is_correct) in &parsed.answers {
        sqlx::query("INSERT INTO answers (question_id, text, is_correct) VALUES (?, ?, ?)")
            .bind(inserted.0)
            .bind(text)
            .bind(if *is_correct { 1 } else { 0 })
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Redirect::to("/admin/questions?msg=Question+créée").into_response())
}

// ===== Questions: edit =======================================================

async fn question_edit_get(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    let row: Option<(i64, String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, statement, kind, explanation, subject_id FROM questions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let (qid, statement, kind, explanation, subject_id) = match row {
        Some(r) => r,
        None => return Err(AppError::bad_request(format!("question {id} introuvable"))),
    };

    let answer_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT text, is_correct FROM answers WHERE question_id = ? ORDER BY id")
            .bind(qid)
            .fetch_all(&state.pool)
            .await?;
    let mut answers: Vec<(String, bool)> =
        answer_rows.into_iter().map(|(t, c)| (t, c == 1)).collect();
    while answers.len() < 6 {
        answers.push((String::new(), false));
    }

    let subjects = list_subjects(&state).await?;
    Ok(render(QuestionFormTemplate {
        title: format!("Modifier la question #{qid}"),
        action: format!("/admin/questions/{qid}/edit"),
        subjects: build_subject_options(subjects, Some(subject_id)),
        statement,
        explanation: explanation.unwrap_or_default(),
        kind,
        answers,
        error: None,
    }))
}

async fn question_edit_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let parsed = parse_question_form(&pairs);
    let subjects = list_subjects(&state).await?;
    let parsed = match parsed {
        Ok(p) => p,
        Err(msg) => {
            let sel: Option<i64> = pair_get(&pairs, "subject_id").parse().ok();
            return Ok(render(QuestionFormTemplate {
                title: format!("Modifier la question #{id}"),
                action: format!("/admin/questions/{id}/edit"),
                subjects: build_subject_options(subjects, sel),
                statement: pair_get(&pairs, "statement"),
                explanation: pair_get(&pairs, "explanation"),
                kind: pair_get_or(&pairs, "kind", "single"),
                answers: collect_answer_pairs(&pairs),
                error: Some(msg),
            }));
        }
    };

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE questions SET subject_id = ?, kind = ?, statement = ?, explanation = ?
         WHERE id = ?",
    )
    .bind(parsed.subject_id)
    .bind(&parsed.kind)
    .bind(&parsed.statement)
    .bind(parsed.explanation.as_ref())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM answers WHERE question_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for (text, is_correct) in &parsed.answers {
        sqlx::query("INSERT INTO answers (question_id, text, is_correct) VALUES (?, ?, ?)")
            .bind(id)
            .bind(text)
            .bind(if *is_correct { 1 } else { 0 })
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Redirect::to("/admin/questions?msg=Question+modifiée").into_response())
}

async fn question_delete_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    sqlx::query("DELETE FROM questions WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/admin/questions?msg=Question+supprimée").into_response())
}

// ===== Subjects ==============================================================

#[derive(Deserialize)]
struct SubjectsQuery {
    msg: Option<String>,
}

async fn subjects_get(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<SubjectsQuery>,
) -> Result<Response, AppError> {
    let rows: Vec<(i64, String, f64, i64, bool)> = sqlx::query_as(
        "SELECT s.id, s.name, s.weight, COUNT(q.id), s.enabled
         FROM subjects s
         LEFT JOIN questions q ON q.subject_id = s.id
         GROUP BY s.id ORDER BY s.name",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(render(SubjectsTemplate {
        subjects: rows,
        flash: q.msg,
    }))
}

async fn subjects_post(
    _: AdminAuth,
    State(state): State<AppState>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    // Cases à cocher : un champ caché "0" précède chaque case "1", donc la
    // dernière valeur reçue pour une clé enabled_<id> donne l'état réel.
    let mut weights: Vec<(i64, f64)> = Vec::new();
    let mut enabled: HashMap<i64, bool> = HashMap::new();
    for (k, v) in &pairs {
        if let Some(suffix) = k.strip_prefix("weight_") {
            let id: i64 = suffix.parse().context("invalid subject id")?;
            let w: f64 = v.parse().context("invalid weight")?;
            // Le poids reste strictement positif ; pour désactiver une
            // matière, on utilise la case « activée ».
            if w > 0.0 {
                weights.push((id, w));
            }
        } else if let Some(suffix) = k.strip_prefix("enabled_") {
            let id: i64 = suffix.parse().context("invalid subject id")?;
            enabled.insert(id, v == "1");
        }
    }

    let mut tx = state.pool.begin().await?;
    for (id, w) in weights {
        sqlx::query("UPDATE subjects SET weight = ? WHERE id = ?")
            .bind(w)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    for (id, en) in enabled {
        sqlx::query("UPDATE subjects SET enabled = ? WHERE id = ?")
            .bind(en)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Redirect::to("/admin/subjects?msg=Matières+enregistrées").into_response())
}

async fn subject_delete_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    // ON DELETE CASCADE supprime aussi les questions de cette matière
    // (et leurs réponses, en cascade à leur tour).
    sqlx::query("DELETE FROM subjects WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/admin/subjects?msg=Matière+supprimée").into_response())
}

// ===== Settings ==============================================================

#[derive(Deserialize)]
struct SettingsQuery {
    msg: Option<String>,
}

async fn settings_get(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<SettingsQuery>,
) -> Result<Response, AppError> {
    Ok(render(SettingsTemplate {
        questions_per_test: read_setting(&state, "questions_per_test", "10").await?,
        pass_threshold_pct: read_setting(&state, "pass_threshold_pct", "70").await?,
        kill_interval_minutes: read_setting(&state, "kill_interval_minutes", "30").await?,
        flash: q.msg,
    }))
}

#[derive(Deserialize)]
struct SettingsForm {
    questions_per_test: i64,
    pass_threshold_pct: f64,
    kill_interval_minutes: i64,
}

async fn settings_post(
    _: AdminAuth,
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    if form.questions_per_test < 1 {
        return Err(AppError::bad_request("questions_per_test doit être ≥ 1"));
    }
    if !(0.0..=100.0).contains(&form.pass_threshold_pct) {
        return Err(AppError::bad_request("seuil hors [0,100]"));
    }
    if form.kill_interval_minutes < 1 {
        return Err(AppError::bad_request("intervalle doit être ≥ 1 minute"));
    }
    let mut tx = state.pool.begin().await?;
    upsert_setting(&mut tx, "questions_per_test", &form.questions_per_test.to_string()).await?;
    upsert_setting(&mut tx, "pass_threshold_pct", &form.pass_threshold_pct.to_string()).await?;
    upsert_setting(&mut tx, "kill_interval_minutes", &form.kill_interval_minutes.to_string()).await?;
    tx.commit().await?;
    Ok(Redirect::to("/admin/settings?msg=Réglages+enregistrés").into_response())
}

// ===== Import ================================================================

#[derive(Deserialize)]
struct ImportQuery {
    msg: Option<String>,
}

async fn import_get(
    _: AdminAuth,
    Query(q): Query<ImportQuery>,
) -> Result<Response, AppError> {
    Ok(render(ImportTemplate {
        flash: q.msg,
        report: None,
    }))
}

async fn import_post(
    _: AdminAuth,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let mut json_body: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart: {e}")))?
    {
        if field.name() == Some("file") {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::bad_request(format!("lecture du fichier: {e}")))?;
            json_body = Some(
                String::from_utf8(bytes.to_vec())
                    .map_err(|e| AppError::bad_request(format!("UTF-8 invalide: {e}")))?,
            );
        }
    }
    let raw = json_body.ok_or_else(|| AppError::bad_request("aucun fichier reçu"))?;
    let parsed: ImportFile =
        serde_json::from_str(&raw).map_err(|e| AppError::bad_request(format!("JSON: {e}")))?;
    let report = importer::import(&state.pool, parsed).await?;

    Ok(render(ImportTemplate {
        flash: None,
        report: Some(ImportReportView {
            subjects_created: report.subjects_created,
            subjects_skipped: report.subjects_skipped,
            questions_imported: report.questions_imported,
            questions_failed: report.questions_failed,
        }),
    }))
}

// ===== History ===============================================================

async fn history_list(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let rows: Vec<(i64, i64, Option<f64>, i64)> = sqlx::query_as(
        "SELECT id, started_at, score_pct, passed FROM attempts
         ORDER BY started_at DESC LIMIT 20",
    )
    .fetch_all(&state.pool)
    .await?;
    let attempts = rows
        .into_iter()
        .map(|(id, ts, score, passed)| HistoryAttemptRow {
            id,
            when: format_ts(ts),
            score_fmt: format!("{:.0}", score.unwrap_or(0.0)),
            passed: passed == 1,
        })
        .collect();
    Ok(render(HistoryListTemplate { attempts }))
}

async fn history_detail(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    let header: Option<(i64, Option<f64>, i64)> =
        sqlx::query_as("SELECT started_at, score_pct, passed FROM attempts WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let (started_at, score, passed) = match header {
        Some(h) => h,
        None => return Err(AppError::bad_request(format!("attempt {id} introuvable"))),
    };

    let rows: Vec<(i64, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT question_id, kind_snapshot, statement_snapshot, answer_text_snapshot,
                was_chosen, is_correct
         FROM attempt_answers
         WHERE attempt_id = ?
         ORDER BY question_id, id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    let mut grouped: Vec<HistoryDetailQuestion> = Vec::new();
    let mut current_qid: Option<i64> = None;
    let mut chosen_set: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut correct_set: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut answer_buf_idx: Vec<(i64, String, bool, bool)> = Vec::new();

    let flush = |grouped: &mut Vec<HistoryDetailQuestion>,
                 stmt: &mut Option<String>,
                 kind: &mut String,
                 answers: &mut Vec<(i64, String, bool, bool)>,
                 chosen: &mut std::collections::HashSet<i64>,
                 correct: &mut std::collections::HashSet<i64>| {
        if let Some(s) = stmt.take() {
            let correct_overall = chosen == correct;
            grouped.push(HistoryDetailQuestion {
                statement: s,
                kind: std::mem::take(kind),
                correct_overall,
                answers: answers
                    .drain(..)
                    .map(|(_, t, was, isc)| HistoryDetailAnswer {
                        text: t,
                        is_correct: isc,
                        was_chosen: was,
                    })
                    .collect(),
            });
            chosen.clear();
            correct.clear();
        }
    };

    let mut current_stmt: Option<String> = None;
    let mut current_kind: String = String::new();

    for (qid, kind, stmt, atext, was, isc) in rows {
        if Some(qid) != current_qid {
            flush(
                &mut grouped,
                &mut current_stmt,
                &mut current_kind,
                &mut answer_buf_idx,
                &mut chosen_set,
                &mut correct_set,
            );
            current_qid = Some(qid);
            current_stmt = Some(stmt);
            current_kind = kind;
        }
        let aid = answer_buf_idx.len() as i64;
        if was == 1 {
            chosen_set.insert(aid);
        }
        if isc == 1 {
            correct_set.insert(aid);
        }
        answer_buf_idx.push((aid, atext, was == 1, isc == 1));
    }
    flush(
        &mut grouped,
        &mut current_stmt,
        &mut current_kind,
        &mut answer_buf_idx,
        &mut chosen_set,
        &mut correct_set,
    );

    Ok(render(HistoryDetailTemplate {
        attempt_id: id,
        when: format_ts(started_at),
        score_fmt: format!("{:.0}", score.unwrap_or(0.0)),
        passed: passed == 1,
        questions: grouped,
    }))
}

// ===== Helpers ===============================================================

struct ParsedQuestion {
    subject_id: i64,
    kind: String,
    statement: String,
    explanation: Option<String>,
    answers: Vec<(String, bool)>,
}

fn parse_question_form(pairs: &[(String, String)]) -> Result<ParsedQuestion, String> {
    let statement = pair_get(pairs, "statement").trim().to_string();
    if statement.is_empty() {
        return Err("L'énoncé ne peut pas être vide.".into());
    }
    let subject_id: i64 = pair_get(pairs, "subject_id")
        .parse()
        .map_err(|_| "Choisis une matière.".to_string())?;
    let kind = pair_get(pairs, "kind");
    if kind != "single" && kind != "multi" {
        return Err("Type invalide (single ou multi).".into());
    }
    let explanation = {
        let s = pair_get(pairs, "explanation").trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    };

    let raw_answers = collect_answer_pairs(pairs);
    let answers: Vec<(String, bool)> = raw_answers
        .into_iter()
        .filter(|(t, _)| !t.trim().is_empty())
        .map(|(t, c)| (t.trim().to_string(), c))
        .collect();

    if answers.len() < 2 {
        return Err("Au moins 2 réponses non vides sont requises.".into());
    }
    let nb_correct = answers.iter().filter(|(_, c)| *c).count();
    let nb_wrong = answers.len() - nb_correct;
    match kind.as_str() {
        "single" => {
            if nb_correct != 1 {
                return Err(format!(
                    "Type 'single' exige exactement 1 réponse correcte (trouvé {nb_correct})."
                ));
            }
        }
        "multi" => {
            if nb_correct < 1 || nb_wrong < 1 {
                return Err("Type 'multi' exige au moins 1 correcte ET 1 incorrecte.".into());
            }
        }
        _ => unreachable!(),
    }

    Ok(ParsedQuestion {
        subject_id,
        kind,
        statement,
        explanation,
        answers,
    })
}

fn collect_answer_pairs(pairs: &[(String, String)]) -> Vec<(String, bool)> {
    let mut texts: HashMap<usize, String> = HashMap::new();
    let mut correct: HashMap<usize, bool> = HashMap::new();
    for (k, v) in pairs {
        if let Some(rest) = k.strip_prefix("ans_") {
            if let Some(idx_str) = rest.strip_suffix("_text") {
                if let Ok(i) = idx_str.parse::<usize>() {
                    texts.insert(i, v.clone());
                }
            } else if let Some(idx_str) = rest.strip_suffix("_correct") {
                if let Ok(i) = idx_str.parse::<usize>() {
                    correct.insert(i, true);
                    let _ = v;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(6);
    for i in 1..=6 {
        out.push((
            texts.remove(&i).unwrap_or_default(),
            correct.remove(&i).unwrap_or(false),
        ));
    }
    out
}

fn pair_get(pairs: &[(String, String)], key: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

fn pair_get_or(pairs: &[(String, String)], key: &str, default: &str) -> String {
    let s = pair_get(pairs, key);
    if s.is_empty() { default.into() } else { s }
}

async fn list_subjects(state: &AppState) -> Result<Vec<(i64, String)>, AppError> {
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM subjects ORDER BY name")
            .fetch_all(&state.pool)
            .await?;
    Ok(rows)
}

async fn read_setting(state: &AppState, key: &str, default: &str) -> Result<String, AppError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.map(|(v,)| v).unwrap_or_else(|| default.to_string()))
}

async fn upsert_setting(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
