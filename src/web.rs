use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
use crate::auth;
use crate::policy::{self, BlockReason, Child, GateDecision};
use crate::quiz::{self, Given, GradedAttempt, QuizQuestion, Submission};

/// Cookie qui retient quel enfant est devant l'écran, posé par le sélecteur de
/// profils (/profiles). Cookie de session : à chaque redémarrage du navigateur
/// (donc du kiosque), on redemande qui est là.
pub const CHILD_COOKIE_NAME: &str = "sesame_child";

/// Commande d'extinction par défaut du bouton « Éteindre ».
///
/// `systemctl poweroff` passe par logind : pour la session LOCALE ACTIVE — et
/// c'est exactement le cas du kiosque —, polkit l'autorise SANS mot de passe
/// (`org.freedesktop.login1.power-off`, `allow_active=yes`). Aucune règle sudo
/// n'est donc requise dans le cas normal.
///
/// Le `|| sudo -n poweroff` est la ceinture-bretelles : si polkit refusait
/// (session jugée inactive, plusieurs sessions ouvertes), la machine porte déjà
/// une règle NOPASSWD sur `poweroff`. Une porte de sortie qui ne s'ouvre pas
/// n'est pas une porte de sortie.
pub const DEFAULT_POWEROFF_CMD: &str = "systemctl poweroff || sudo -n poweroff";

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    /// Ligne de commande shell exécutée par `POST /poweroff`. Résolue une fois
    /// au démarrage depuis `[kiosk] poweroff`, ou [`DEFAULT_POWEROFF_CMD`].
    pub poweroff_cmd: String,
    /// Mode parent : l'adulte a déverrouillé la machine avec le mot de passe
    /// d'administration. Tant qu'il est actif, /api/gate répond « ouvert » sans
    /// enfant : rien n'est décompté, rien n'expire, le minuteur ne coupe pas.
    ///
    /// EN MÉMOIRE, jamais en base — et c'est le cœur du contrat. Le serveur
    /// naît et meurt avec la session (sesame-session le lance puis le tue) :
    /// fermer la session ou redémarrer réarme donc la porte PAR CONSTRUCTION.
    /// En base, un parent oublieux laisserait la machine ouverte pour toujours.
    pub parent_mode: Arc<AtomicBool>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/profiles", get(profiles_get))
        .route("/profile", post(profile_post))
        .route("/submit", post(submit))
        .route("/unlock", post(unlock))
        .route("/poweroff", post(poweroff))
        .route("/parent-unlock", post(parent_unlock))
        .route("/api/gate", get(api_gate))
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
    /// La page se recharge à l'expiration : le moteur décide de la suite
    /// (nouveau contrôle, budget épuisé, couvre-feu).
    refresh_secs: i64,
}

#[derive(Template)]
#[template(path = "blocked.html")]
struct BlockedTemplate {
    child_name: String,
    child_avatar: String,
    emoji: String,
    title: String,
    detail: String,
    /// Message d'erreur du coin parent (mot de passe refusé). Vide = pas
    /// d'erreur ; le gabarit rouvre le panneau quand il est non vide.
    parent_error: String,
}

#[derive(Template)]
#[template(path = "profiles.html")]
struct ProfilesTemplate {
    children: Vec<Child>,
}

#[derive(Template)]
#[template(path = "poweroff.html")]
struct PoweroffTemplate;

#[derive(Template)]
#[template(path = "parent.html")]
struct ParentModeTemplate;

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
            refresh_secs: remaining_secs + 3,
        })),

        GateDecision::Blocked { reason } => Ok(render(blocked_page(&child, &reason))),

        GateDecision::ExamAvailable {
            questions,
            threshold_pct,
            max_grant_minutes,
        } => {
            let questions = quiz::pick_questions(
                &state.pool,
                child.id,
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
                    parent_error: String::new(),
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
        policy::open_grant(&state.pool, &child, Some(form.attempt_id), None, false).await?;
    }

    Ok(Redirect::to("/").into_response())
}

/// Sortie de secours de l'enfant : **éteindre la machine** plutôt que de passer
/// le contrôle. C'est sans danger, et voici pourquoi : éteindre ne DÉBLOQUE
/// rien. La porte protège le bureau, pas l'interrupteur ; un redémarrage
/// ramène à cette même page, aucun temps gagné, aucun temps perdu (le budget
/// est un compteur, pas une horloge d'expiration). L'offrir à l'enfant ne crée
/// donc aucune faille — ça bouche seulement un piège : rester coincé devant un
/// contrôle qu'il ne veut pas faire, sans même pouvoir arrêter l'ordinateur.
///
/// La commande tourne DÉTACHÉE (`spawn`, jamais `status`) : on ne l'attend pas.
/// L'attendre, ce serait risquer que la machine s'éteigne sous nos pieds avant
/// que la page « À bientôt » n'atteigne le navigateur. Le shell porte le
/// `||` du repli sudo — d'où `sh -c`, sur une chaîne fixe, sans entrée externe.
async fn poweroff(State(state): State<AppState>) -> Result<Response, AppError> {
    tracing::info!(cmd = %state.poweroff_cmd, "extinction demandée depuis la porte");
    std::process::Command::new("sh")
        .arg("-c")
        .arg(&state.poweroff_cmd)
        .spawn()
        .context("lancement de la commande d'extinction")?;
    Ok(render(PoweroffTemplate))
}

#[derive(Deserialize)]
struct ParentUnlockForm {
    password: String,
}

/// Coin parent de la page de blocage : ouvrir la machine avec le mot de passe
/// d'administration, sans quitter la porte. Pendant un couvre-feu, le bureau
/// n'existe pas et /admin n'est pas affiché : sans ce bouton, l'adulte n'a que
/// la console VT.
///
/// Ce n'est PAS une concession : aucun grant, aucun décompte, rien d'imputé au
/// budget d'un enfant — l'adulte n'emprunte pas le temps de quelqu'un d'autre.
/// On lève le [`AppState::parent_mode`], et la machine est à lui jusqu'à ce
/// qu'il ferme la session ou redémarre : c'est SA responsabilité, le
/// formulaire le lui dit en toutes lettres.
async fn parent_unlock(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(form): Form<ParentUnlockForm>,
) -> Result<Response, AppError> {
    // Pas de mot de passe configuré → pas de déblocage par ici : ce serait un
    // bouton « ouvrir » à la portée de l'enfant.
    let ok = match auth::read_password_hash(&state.pool).await?.as_deref() {
        Some(hash) if !hash.is_empty() => auth::verify_password(&form.password, hash),
        _ => false,
    };

    if !ok {
        // Frein à la force brute : argon2 est déjà lent, on ajoute une pause
        // fixe. Un enfant qui essaie des mots de passe attendra 2 s par essai.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        tracing::warn!("coin parent : mot de passe refusé");

        // On remontre l'écran d'où il vient. S'il n'est plus bloqué (la
        // situation a pu changer pendant la pause), l'accueil décidera.
        let Some(child) = resolve_child(&state, &cookies).await? else {
            return Ok(Redirect::to("/profiles").into_response());
        };
        let GateDecision::Blocked { reason } = policy::evaluate(&state.pool, &child).await?
        else {
            return Ok(Redirect::to("/").into_response());
        };
        let mut page = blocked_page(&child, &reason);
        page.parent_error = "Mot de passe incorrect.".to_string();
        return Ok(render(page));
    }

    state.parent_mode.store(true, Ordering::Relaxed);
    tracing::info!("mode parent activé depuis la porte — aucun décompte jusqu'à la fin de session");
    Ok(render(ParentModeTemplate))
}

// ===== API (kiosque, fenêtre de verrouillage, minuteur) =====

/// *La MACHINE est-elle ouverte ?* — et non « cet enfant a-t-il du temps ? ».
///
/// Le kiosque et le minuteur n'ont pas de cookie : ils ne savent pas QUI est
/// assis devant l'écran (c'est l'enfant qui l'a choisi dans le navigateur).
/// La seule question qui a un sens pour eux est : *quelqu'un a-t-il gagné
/// l'ordinateur ?* — s'il y a une concession vivante, la porte s'ouvre, et le
/// temps est débité à celui qui l'a gagnée.
#[derive(Serialize)]
struct GateResponse {
    unlocked: bool,
    child_id: Option<i64>,
    child_name: Option<String>,
    remaining_secs: i64,
    /// Ce que le minuteur doit faire quand le temps expire : `overlay` ou
    /// `logout`. C'est un réglage du parent — le minuteur ne le devine pas, il
    /// le demande.
    lock_mode: String,
}

async fn api_gate(State(state): State<AppState>) -> Result<Json<GateResponse>, AppError> {
    let lock_mode: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'lock_mode'")
            .fetch_optional(&state.pool)
            .await?;
    let lock_mode = lock_mode.map(|(v,)| v).unwrap_or_else(|| "logout".into());

    // Mode parent : ouvert, mais SANS enfant. Le minuteur ne trouve pas de
    // child_id → il ne débite rien et ne coupe jamais ; le grand livre des
    // enfants n'est pas touché. La responsabilité du temps est à l'adulte.
    if state.parent_mode.load(Ordering::Relaxed) {
        return Ok(Json(GateResponse {
            unlocked: true,
            child_id: None,
            child_name: Some("le parent".to_string()),
            remaining_secs: 0,
            lock_mode,
        }));
    }

    for child in policy::list_children(&state.pool).await? {
        if let GateDecision::Granted { remaining_secs } =
            policy::evaluate(&state.pool, &child).await?
        {
            return Ok(Json(GateResponse {
                unlocked: true,
                child_id: Some(child.id),
                child_name: Some(child.name),
                remaining_secs,
                lock_mode,
            }));
        }
    }

    Ok(Json(GateResponse {
        unlocked: false,
        child_id: None,
        child_name: None,
        remaining_secs: 0,
        lock_mode,
    }))
}

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
        parent_error: String::new(),
    }
}

// ===== Form parsing =====

/// Deux préfixes, et ce n'est pas cosmétique : `q_` porte des identifiants de
/// réponse, `t_` du texte libre. Les confondre serait fatal — la réponse « 8 »
/// d'une addition se parserait en `answer_id = 8`, et corrigerait la question
/// contre une option d'un tout autre QCM. Le préfixe rend l'ambiguïté impossible
/// à écrire.
fn parse_form(pairs: &[(String, String)]) -> Result<(Vec<i64>, Submission), AppError> {
    let mut question_ids: Vec<i64> = Vec::new();
    let mut given: Submission = HashMap::new();

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
            match given.entry(qid).or_default() {
                Given::Choices(ids) => ids.push(aid),
                // Texte ET cases pour la même question : formulaire trafiqué.
                // On ignore ici, `grade` tranchera contre le `kind` en base.
                Given::Text(_) => {}
            }
        } else if let Some(suffix) = k.strip_prefix("t_") {
            let qid: i64 = suffix
                .parse()
                .with_context(|| format!("invalid t_ key '{suffix}'"))?;
            given.insert(qid, Given::Text(v.clone()));
        }
    }

    if question_ids.is_empty() {
        return Err(AppError::bad_request("aucune question soumise"));
    }
    Ok((question_ids, given))
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
                    answer_id, answer_text_snapshot, given_text_snapshot,
                    was_chosen, is_correct)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(attempt_id)
            .bind(q.question_id)
            .bind(&q.kind)
            .bind(&q.statement)
            .bind(a.answer_id)
            .bind(&a.text)
            .bind(q.given_text.as_ref())
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
