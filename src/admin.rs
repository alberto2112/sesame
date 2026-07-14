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
use crate::dedup;
use crate::importer::{self, ImportFile};
use crate::policy;
use crate::web::{AppError, AppState, render};

// ===== Router ================================================================

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/setup", get(setup_get).post(setup_post))
        .route("/login", get(login_get).post(login_post))
        .route("/logout", post(logout_post))
        .route("/questions", get(questions_list))
        .route("/questions/dedupe", post(questions_dedupe_post))
        .route("/questions/difficulty", post(questions_difficulty_post))
        .route("/questions/new", get(question_new_get).post(question_new_post))
        .route("/questions/:id/edit", get(question_edit_get).post(question_edit_post))
        .route("/questions/:id/delete", post(question_delete_post))
        .route("/children", get(children_list).post(child_create_post))
        .route("/children/:id/edit", get(child_edit_get).post(child_edit_post))
        .route("/children/:id/delete", post(child_delete_post))
        .route("/children/:id/grant", post(child_grant_post))
        .route("/children/:id/revoke", post(child_revoke_post))
        .route("/children/:id/schedules", post(schedule_add_post))
        .route("/children/:id/schedules/:sid/delete", post(schedule_delete_post))
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
    /// Groupes de doublons — montrés AVANT de supprimer quoi que ce soit. Un
    /// bouton qui efface sans dire quoi n'est pas un outil, c'est un pari.
    duplicates: Vec<DuplicateRow>,
    /// Total des questions qui disparaîtraient.
    duplicate_count: usize,
    /// Le filtre courant, renvoyé tel quel pour y revenir après un lot.
    filter_subject: Option<i64>,
    filter_difficulty: Option<i64>,
    /// (valeur, sélectionnée) — le booléen est calculé ICI, pas dans le gabarit :
    /// Askama compare mal une liaison déréférencée à un champ (cf. CLAUDE.md).
    difficulty_opts: Vec<(i64, bool)>,
    flash: Option<String>,
}

/// Un groupe de doublons, aplati pour le gabarit.
struct DuplicateRow {
    statement: String,
    keep_id: i64,
    /// « #412 (Sciences) », « #98 (Sciences, 3 contrôles) » — l'historique est
    /// signalé : c'est lui qui sera réaffecté au survivant.
    victims: Vec<String>,
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
    difficulty: i64,
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
    difficulty: i64,
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
    session_minutes: String,
    lock_mode: String,
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
    children: Vec<ChildOption>,
}

struct ChildOption {
    id: i64,
    label: String,
    selected: bool,
}

struct HistoryAttemptRow {
    id: i64,
    when: String,
    child: String,
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
    /// Filtre par difficulté. Sert surtout à retrouver le lot des questions
    /// jamais notées : importées sans `difficulty`, elles valent toutes 3 par
    /// défaut (importer.rs). Un 3 « par défaut » et un 3 « choisi » se
    /// ressemblent en base — c'est en les filtrant qu'on les retrouve.
    difficulty: Option<i64>,
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

    // Deux filtres facultatifs = quatre combinaisons. Plutôt que quatre requêtes
    // recopiées, on construit le WHERE : les valeurs restent liées (`bind`),
    // seuls les fragments SQL sont concaténés — aucune donnée ne touche le SQL.
    let mut sql = String::from(
        "SELECT q.id, q.statement, q.kind, s.name, q.difficulty, COUNT(a.id)
         FROM questions q
         JOIN subjects s ON s.id = q.subject_id
         LEFT JOIN answers a ON a.question_id = q.id",
    );
    let mut conds: Vec<&str> = Vec::new();
    if q.subject.is_some() {
        conds.push("q.subject_id = ?");
    }
    if q.difficulty.is_some() {
        conds.push("q.difficulty = ?");
    }
    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }
    sql.push_str(" GROUP BY q.id ORDER BY q.id DESC");

    let mut query = sqlx::query_as::<_, (i64, String, String, String, i64, i64)>(&sql);
    if let Some(sid) = q.subject {
        query = query.bind(sid);
    }
    if let Some(d) = q.difficulty {
        query = query.bind(d);
    }
    let rows = query.fetch_all(&state.pool).await?;

    let questions = rows
        .into_iter()
        .map(|(id, st, kd, sn, df, na)| QuestionRow {
            id,
            statement: st,
            kind: kd,
            subject_name: sn,
            difficulty: df,
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

    // Les doublons se cherchent sur TOUTE la banque, jamais dans le filtre de
    // matière courant : deux questions au même énoncé dans deux matières
    // différentes restent un doublon, et c'est même le cas le plus sournois.
    let groups = dedup::find_duplicates(&state.pool).await?;
    let duplicate_count = groups.iter().map(|g| g.victims.len()).sum();
    let duplicates = groups
        .into_iter()
        .map(|g| DuplicateRow {
            statement: g.statement,
            keep_id: g.keep_id,
            victims: g
                .victims
                .iter()
                .map(|v| match v.history_count {
                    0 => format!("#{} ({})", v.id, v.subject_name),
                    n => format!("#{} ({}, {} contrôle(s))", v.id, v.subject_name, n),
                })
                .collect(),
        })
        .collect();

    Ok(render(QuestionsListTemplate {
        questions,
        subjects: subject_opts,
        // Renvoyés au gabarit pour que l'action en lot revienne sur le MÊME
        // filtre : on note une matière entière en plusieurs passes, ce serait
        // absurde de repartir de la liste complète à chaque enregistrement.
        filter_subject: q.subject,
        filter_difficulty: q.difficulty,
        difficulty_opts: (1..=5).map(|d| (d, q.difficulty == Some(d))).collect(),
        duplicates,
        duplicate_count,
        flash: q.msg,
    }))
}

/// Note plusieurs questions d'un coup. Sans ça, les 390 questions importées
/// sans `difficulty` (toutes à 3 par défaut) se corrigeraient une par une : 390
/// formulaires. Ce n'est pas du travail, c'est une punition.
async fn questions_difficulty_post(
    _: AdminAuth,
    State(state): State<AppState>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let difficulty: i64 = pair_get(&pairs, "difficulty")
        .parse()
        .map_err(|_| AppError::bad_request("difficulté invalide"))?;
    if !(1..=5).contains(&difficulty) {
        return Err(AppError::bad_request("la difficulté doit être entre 1 et 5"));
    }

    let ids: Vec<i64> = pairs
        .iter()
        .filter(|(k, _)| k == "ids")
        .filter_map(|(_, v)| v.parse::<i64>().ok())
        .collect();

    // Le filtre courant est réinjecté dans la redirection : on revient là où on
    // était, pas en haut d'une liste de 3000 lignes.
    let back = {
        let mut q: Vec<String> = Vec::new();
        if let Some(s) = pair_get_opt(&pairs, "filter_subject") {
            q.push(format!("subject={s}"));
        }
        if let Some(d) = pair_get_opt(&pairs, "filter_difficulty") {
            q.push(format!("difficulty={d}"));
        }
        q
    };
    let query_of = |msg: String| {
        let mut parts = back.clone();
        parts.push(format!("msg={msg}"));
        format!("/admin/questions?{}", parts.join("&"))
    };

    if ids.is_empty() {
        return Ok(Redirect::to(&query_of("Aucune+question+sélectionnée".into())).into_response());
    }

    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("UPDATE questions SET difficulty = ? WHERE id IN ({placeholders})");
    let mut query = sqlx::query(&sql).bind(difficulty);
    for id in &ids {
        query = query.bind(id);
    }
    let updated = query.execute(&state.pool).await?.rows_affected();

    Ok(Redirect::to(&query_of(format!(
        "{updated}+question(s)+notée(s)+en+difficulté+{difficulty}"
    )))
    .into_response())
}

/// Supprime les doublons : le plus RÉCENT de chaque groupe survit, l'historique
/// des autres lui est réaffecté. Toute la logique est dans `dedup` — un handler
/// ne décide de rien.
async fn questions_dedupe_post(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let r = dedup::purge(&state.pool).await?;
    let msg = format!(
        "{}+doublon(s)+supprimé(s)+dans+{}+groupe(s)+—+{}+ligne(s)+d'historique+réaffectée(s)",
        r.deleted, r.groups, r.repointed
    );
    Ok(Redirect::to(&format!("/admin/questions?msg={msg}")).into_response())
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
        difficulty: 3,
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
                difficulty: pair_get_or(&pairs, "difficulty", "3").parse().unwrap_or(3),
                answers: collect_answer_pairs(&pairs),
                error: Some(msg),
            }));
        }
    };

    let now = chrono::Utc::now().timestamp();
    let mut tx = state.pool.begin().await?;
    let inserted: (i64,) = sqlx::query_as(
        "INSERT INTO questions (subject_id, kind, statement, explanation, difficulty, created_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(parsed.subject_id)
    .bind(&parsed.kind)
    .bind(&parsed.statement)
    .bind(parsed.explanation.as_ref())
    .bind(parsed.difficulty)
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
    let row: Option<(i64, String, String, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT id, statement, kind, explanation, subject_id, difficulty
         FROM questions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let (qid, statement, kind, explanation, subject_id, difficulty) = match row {
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
        difficulty,
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
                difficulty: pair_get_or(&pairs, "difficulty", "3").parse().unwrap_or(3),
                answers: collect_answer_pairs(&pairs),
                error: Some(msg),
            }));
        }
    };

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE questions SET subject_id = ?, kind = ?, statement = ?, explanation = ?,
                              difficulty = ?
         WHERE id = ?",
    )
    .bind(parsed.subject_id)
    .bind(&parsed.kind)
    .bind(&parsed.statement)
    .bind(parsed.explanation.as_ref())
    .bind(parsed.difficulty)
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

/// Supprime toutes les questions en double — toutes matières confondues.
/// Le doublon est défini par un `statement` identique ; on conserve la
/// question la plus ancienne (`id` minimal) de chaque groupe et on efface
/// les autres. Les réponses partent en cascade (`ON DELETE CASCADE`).
// ===== Enfants ===============================================================

#[derive(Template)]
#[template(path = "admin/children.html")]
struct ChildrenTemplate {
    children: Vec<ChildRow>,
    flash: Option<String>,
}

/// Ce que la LISTE montre : l'état, pas les réglages. La difficulté et la
/// durée de session vivent dans la fiche de l'enfant, pas ici.
struct ChildRow {
    id: i64,
    name: String,
    avatar: String,
    enabled: bool,
    daily_budget_minutes: i64,
    used_today_min: i64,
    /// Questions visibles pour sa plage de difficulté — 0 = examen impossible.
    available_questions: i64,
    attempts_passed: i64,
    attempts_total: i64,
    /// Concession vivante (temps en cours) ?
    has_grant: bool,
}

#[derive(Template)]
#[template(path = "admin/child_form.html")]
struct ChildFormTemplate {
    child_id: i64,
    name: String,
    avatar: String,
    enabled: bool,
    schedules: Vec<ScheduleRow>,
    difficulty_min: i64,
    difficulty_max: i64,
    questions_per_test: i64,
    pass_threshold_pct: String,
    session_minutes: i64,
    daily_budget_minutes: i64,
    weekend_budget_minutes: i64,
    exam_cooldown_minutes: i64,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ChildrenQuery {
    msg: Option<String>,
}

async fn children_list(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ChildrenQuery>,
) -> Result<Response, AppError> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let rows: Vec<(i64, String, String, i64, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT c.id, c.name, c.avatar, c.enabled, c.daily_budget_minutes,
                COALESCE(u.consumed_secs, 0) / 60,
                (SELECT COUNT(*) FROM questions q
                  JOIN subjects s ON s.id = q.subject_id AND s.enabled = 1
                  WHERE q.difficulty BETWEEN c.difficulty_min AND c.difficulty_max),
                (SELECT COUNT(*) FROM attempts a WHERE a.child_id = c.id AND a.passed = 1),
                (SELECT COUNT(*) FROM attempts a WHERE a.child_id = c.id),
                EXISTS(SELECT 1 FROM grants g WHERE g.child_id = c.id AND g.closed_at IS NULL)
         FROM children c
         LEFT JOIN daily_usage u ON u.child_id = c.id AND u.day = ?
         ORDER BY c.position, c.id",
    )
    .bind(&today)
    .fetch_all(&state.pool)
    .await?;

    let children = rows
        .into_iter()
        .map(
            |(id, name, avatar, enabled, budget, used, avail, passed, total, grant)| ChildRow {
                id,
                name,
                avatar,
                enabled: enabled == 1,
                daily_budget_minutes: budget,
                used_today_min: used,
                available_questions: avail,
                attempts_passed: passed,
                attempts_total: total,
                has_grant: grant == 1,
            },
        )
        .collect();

    Ok(render(ChildrenTemplate {
        children,
        flash: q.msg,
    }))
}

#[derive(Deserialize)]
struct ChildCreateForm {
    name: String,
    avatar: String,
}

/// Création volontairement minimale (nom + emoji) : l'enfant naît avec les
/// défauts du schéma, le réglage fin se fait dans sa fiche.
async fn child_create_post(
    _: AdminAuth,
    State(state): State<AppState>,
    Form(form): Form<ChildCreateForm>,
) -> Result<Response, AppError> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("le prénom ne peut pas être vide"));
    }
    let avatar = if form.avatar.trim().is_empty() {
        "🙂".to_string()
    } else {
        form.avatar.trim().to_string()
    };

    let dup: Option<(i64,)> = sqlx::query_as("SELECT id FROM children WHERE name = ?")
        .bind(name)
        .fetch_optional(&state.pool)
        .await?;
    if dup.is_some() {
        return Err(AppError::bad_request(format!("« {name} » existe déjà")));
    }

    sqlx::query("INSERT INTO children (name, avatar) VALUES (?, ?)")
        .bind(name)
        .bind(&avatar)
        .execute(&state.pool)
        .await?;

    Ok(Redirect::to("/admin/children?msg=Enfant+créé").into_response())
}

async fn child_edit_get(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    let row: Option<(String, String, i64, i64, i64, i64, f64, i64, i64, i64, i64)> =
        sqlx::query_as(
            "SELECT name, avatar, enabled, difficulty_min, difficulty_max,
                    questions_per_test, pass_threshold_pct, session_minutes,
                    daily_budget_minutes, weekend_budget_minutes, exam_cooldown_minutes
             FROM children WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;

    let Some((name, avatar, enabled, dmin, dmax, qpt, threshold, session, daily, weekend, cooldown)) =
        row
    else {
        return Err(AppError::bad_request(format!("enfant {id} introuvable")));
    };

    Ok(render(ChildFormTemplate {
        child_id: id,
        name,
        avatar,
        enabled: enabled == 1,
        schedules: load_schedules(&state, id).await?,
        difficulty_min: dmin,
        difficulty_max: dmax,
        questions_per_test: qpt,
        pass_threshold_pct: format!("{threshold:.0}"),
        session_minutes: session,
        daily_budget_minutes: daily,
        weekend_budget_minutes: weekend,
        exam_cooldown_minutes: cooldown,
        error: None,
    }))
}

#[derive(Deserialize)]
struct ChildEditForm {
    name: String,
    avatar: String,
    #[serde(default)]
    enabled: Option<String>,
    difficulty_min: i64,
    difficulty_max: i64,
    questions_per_test: i64,
    pass_threshold_pct: f64,
    session_minutes: i64,
    daily_budget_minutes: i64,
    weekend_budget_minutes: i64,
    exam_cooldown_minutes: i64,
}

async fn child_edit_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
    Form(form): Form<ChildEditForm>,
) -> Result<Response, AppError> {
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("le prénom ne peut pas être vide"));
    }
    if !(1..=5).contains(&form.difficulty_min)
        || !(1..=5).contains(&form.difficulty_max)
        || form.difficulty_min > form.difficulty_max
    {
        return Err(AppError::bad_request(
            "plage de difficulté invalide (1 ≤ min ≤ max ≤ 5)",
        ));
    }
    if form.questions_per_test < 1 {
        return Err(AppError::bad_request("au moins 1 question par contrôle"));
    }
    if !(0.0..=100.0).contains(&form.pass_threshold_pct) {
        return Err(AppError::bad_request("seuil hors [0,100]"));
    }
    if form.session_minutes < 1 {
        return Err(AppError::bad_request("la session doit durer ≥ 1 minute"));
    }
    if form.daily_budget_minutes < 0
        || form.weekend_budget_minutes < 0
        || form.exam_cooldown_minutes < 0
    {
        return Err(AppError::bad_request("les budgets ne peuvent pas être négatifs"));
    }

    let avatar = if form.avatar.trim().is_empty() {
        "🙂".to_string()
    } else {
        form.avatar.trim().to_string()
    };
    let enabled = if form.enabled.is_some() { 1 } else { 0 };

    sqlx::query(
        "UPDATE children SET name = ?, avatar = ?, enabled = ?,
                difficulty_min = ?, difficulty_max = ?, questions_per_test = ?,
                pass_threshold_pct = ?, session_minutes = ?, daily_budget_minutes = ?,
                weekend_budget_minutes = ?, exam_cooldown_minutes = ?
         WHERE id = ?",
    )
    .bind(&name)
    .bind(&avatar)
    .bind(enabled)
    .bind(form.difficulty_min)
    .bind(form.difficulty_max)
    .bind(form.questions_per_test)
    .bind(form.pass_threshold_pct)
    .bind(form.session_minutes)
    .bind(form.daily_budget_minutes)
    .bind(form.weekend_budget_minutes)
    .bind(form.exam_cooldown_minutes)
    .bind(id)
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to("/admin/children?msg=Enfant+enregistré").into_response())
}

/// Supprime un enfant. Ses tentatives passées sont anonymisées (child_id NULL)
/// plutôt que supprimées : l'historique pédagogique survit. Concessions,
/// consommation et plages horaires partent en cascade.
async fn child_delete_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE attempts SET child_id = NULL WHERE child_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM children WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Redirect::to("/admin/children?msg=Enfant+supprimé").into_response())
}

// ===== Bouton de secours du parent ===========================================

#[derive(Deserialize)]
struct GrantForm {
    minutes: i64,
}

/// Donne du temps SANS contrôle. Décision du parent : la concession ignore le
/// budget quotidien et le couvre-feu (bypass) — sinon ce bouton serait inutile
/// précisément quand on en a besoin (devoir à finir, budget épuisé).
async fn child_grant_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
    Form(form): Form<GrantForm>,
) -> Result<Response, AppError> {
    if !(1..=600).contains(&form.minutes) {
        return Err(AppError::bad_request("minutes hors de [1, 600]"));
    }
    let child = policy::load_child(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::bad_request("enfant inconnu ou désactivé"))?;

    policy::open_grant(&state.pool, &child, None, Some(form.minutes), true).await?;
    tracing::info!(child = %child.name, minutes = form.minutes, "concession parentale");

    Ok(Redirect::to("/admin/children?msg=Temps+accordé").into_response())
}

/// Coupe le temps en cours, tout de suite. L'écran de l'enfant repassera au
/// contrôle (ou au blocage) à la prochaine évaluation.
async fn child_revoke_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, AppError> {
    policy::close_open_grants(&state.pool, id).await?;
    Ok(Redirect::to("/admin/children?msg=Temps+retiré").into_response())
}

// ===== Plages horaires (admin) ==============================================

struct ScheduleRow {
    id: i64,
    weekday_name: &'static str,
    start_fmt: String,
    end_fmt: String,
}

const WEEKDAYS_FR: [&str; 7] = [
    "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
];

async fn load_schedules(state: &AppState, child_id: i64) -> Result<Vec<ScheduleRow>, AppError> {
    let rows: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, weekday, start_min, end_min FROM schedules
         WHERE child_id = ? ORDER BY weekday, start_min",
    )
    .bind(child_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, wd, start, end)| ScheduleRow {
            id,
            weekday_name: WEEKDAYS_FR[(wd.clamp(0, 6)) as usize],
            start_fmt: format!("{:02}:{:02}", start / 60, start % 60),
            end_fmt: format!("{:02}:{:02}", end / 60, end % 60),
        })
        .collect())
}

/// « HH:MM » → minutes depuis minuit.
fn parse_hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    if (0..=24).contains(&h) && (0..=59).contains(&m) && h * 60 + m <= 1440 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Une seule saisie (jours cochés + plage horaire) crée une ligne PAR jour
/// coché : le cas courant « lundi-vendredi 17h-19h » se fait en un geste.
async fn schedule_add_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath(id): AxPath<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let days: Vec<i64> = pairs
        .iter()
        .filter(|(k, _)| k == "day")
        .filter_map(|(_, v)| v.parse().ok())
        .filter(|d| (0..=6).contains(d))
        .collect();
    if days.is_empty() {
        return Err(AppError::bad_request("coche au moins un jour"));
    }

    let start = parse_hhmm(&pair_get(&pairs, "start"))
        .ok_or_else(|| AppError::bad_request("heure de début invalide (HH:MM)"))?;
    let end = parse_hhmm(&pair_get(&pairs, "end"))
        .ok_or_else(|| AppError::bad_request("heure de fin invalide (HH:MM)"))?;
    if start >= end {
        return Err(AppError::bad_request("le début doit précéder la fin"));
    }

    let mut tx = state.pool.begin().await?;
    for day in &days {
        sqlx::query(
            "INSERT INTO schedules (child_id, weekday, start_min, end_min)
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(day)
        .bind(start)
        .bind(end)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(Redirect::to(&format!("/admin/children/{id}/edit")).into_response())
}

async fn schedule_delete_post(
    _: AdminAuth,
    State(state): State<AppState>,
    AxPath((id, sid)): AxPath<(i64, i64)>,
) -> Result<Response, AppError> {
    sqlx::query("DELETE FROM schedules WHERE id = ? AND child_id = ?")
        .bind(sid)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!("/admin/children/{id}/edit")).into_response())
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
        session_minutes: read_setting(&state, "session_minutes", "30").await?,
        lock_mode: read_setting(&state, "lock_mode", "overlay").await?,
        flash: q.msg,
    }))
}

#[derive(Deserialize)]
struct SettingsForm {
    questions_per_test: i64,
    pass_threshold_pct: f64,
    session_minutes: i64,
    lock_mode: String,
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
    if form.session_minutes < 1 {
        return Err(AppError::bad_request("la session doit durer ≥ 1 minute"));
    }
    if !matches!(form.lock_mode.as_str(), "overlay" | "logout") {
        return Err(AppError::bad_request("mode de verrouillage inconnu"));
    }
    let mut tx = state.pool.begin().await?;
    upsert_setting(&mut tx, "questions_per_test", &form.questions_per_test.to_string()).await?;
    upsert_setting(&mut tx, "pass_threshold_pct", &form.pass_threshold_pct.to_string()).await?;
    upsert_setting(&mut tx, "session_minutes", &form.session_minutes.to_string()).await?;
    upsert_setting(&mut tx, "lock_mode", &form.lock_mode).await?;
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

#[derive(Deserialize)]
struct HistoryQuery {
    child: Option<i64>,
}

async fn history_list(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Response, AppError> {
    let base = "SELECT a.id, a.started_at, a.score_pct, a.passed,
                       COALESCE(c.avatar || ' ' || c.name, '—')
                FROM attempts a
                LEFT JOIN children c ON c.id = a.child_id";

    let rows: Vec<(i64, i64, Option<f64>, i64, String)> = if let Some(cid) = q.child {
        sqlx::query_as(&format!(
            "{base} WHERE a.child_id = ? ORDER BY a.started_at DESC LIMIT 20"
        ))
        .bind(cid)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as(&format!("{base} ORDER BY a.started_at DESC LIMIT 20"))
            .fetch_all(&state.pool)
            .await?
    };

    let attempts = rows
        .into_iter()
        .map(|(id, ts, score, passed, child)| HistoryAttemptRow {
            id,
            when: format_ts(ts),
            child,
            score_fmt: format!("{:.0}", score.unwrap_or(0.0)),
            passed: passed == 1,
        })
        .collect();

    let kid_rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, avatar, name FROM children ORDER BY position, id")
            .fetch_all(&state.pool)
            .await?;
    let children = kid_rows
        .into_iter()
        .map(|(id, avatar, name)| ChildOption {
            id,
            label: format!("{avatar} {name}"),
            selected: q.child == Some(id),
        })
        .collect();

    Ok(render(HistoryListTemplate { attempts, children }))
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
    difficulty: i64,
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
    if !matches!(kind.as_str(), "single" | "multi" | "exact" | "number") {
        return Err("Type invalide (single, multi, exact ou number).".into());
    }
    let difficulty: i64 = pair_get_or(pairs, "difficulty", "3")
        .parse()
        .map_err(|_| "Difficulté invalide.".to_string())?;
    if !(1..=5).contains(&difficulty) {
        return Err("La difficulté doit être entre 1 et 5.".into());
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

    let nb_correct = answers.iter().filter(|(_, c)| *c).count();
    let nb_wrong = answers.len() - nb_correct;

    // 'exact'/'number' : une seule réponse, la bonne. Pas d'options à proposer —
    // l'enfant l'écrit. Mêmes règles que l'importeur (importer.rs).
    if crate::quiz::is_free_input(&kind) {
        if answers.len() != 1 || nb_correct != 1 {
            return Err(format!(
                "Type '{kind}' : remplis UNE seule réponse (la bonne) et coche « correcte »."
            ));
        }
        if kind == "number" && crate::quiz::parse_number(&answers[0].0).is_none() {
            return Err(format!(
                "Type 'number' : « {} » n'est pas un nombre.",
                answers[0].0
            ));
        }
        return Ok(ParsedQuestion {
            subject_id,
            kind,
            statement,
            explanation,
            difficulty,
            answers,
        });
    }

    if answers.len() < 2 {
        return Err("Au moins 2 réponses non vides sont requises.".into());
    }
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
        difficulty,
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

/// Comme `pair_get`, mais distingue « absent » de « vide » : un filtre non posé
/// ne doit pas repartir dans l'URL sous la forme `subject=`.
fn pair_get_opt(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
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
