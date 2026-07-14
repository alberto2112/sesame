use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use sqlx::SqlitePool;
use tracing_subscriber::EnvFilter;

use sesame::config::Config;
use sesame::quiz::Submission;
use sesame::{auth, db, importer, policy, quiz, web};

const HELP: &str = "\
sesame — portail de contrôle avant d'utiliser l'ordinateur

USAGE :
    sesame [OPTIONS] [COMMANDE]

COMMANDES :
    (aucune)            Démarre le serveur. Ouvre l'examen, ou la configuration
                        admin si aucun mot de passe administrateur n'existe.
    admin               Démarre le serveur et ouvre directement /admin.
    import <fichier>    Importe des questions depuis un fichier JSON.
    preview [n]         Simulation console d'un contrôle (outil d'admin).

OPTIONS :
    -c, --config <fichier>   Utilise ce fichier de configuration.
    -h, --help               Affiche cette aide.

CONFIGURATION :
    Sans --config, ces emplacements sont essayés dans l'ordre ; le premier
    qui existe est utilisé :
      1. <répertoire de config du système>/sesame/config.toml
      2. ./config.toml  (répertoire courant)
";

/// Commande à exécuter, telle que résolue depuis la ligne de commande.
enum Command {
    Server { force_admin: bool },
    Import { path: PathBuf },
    Preview { count: Option<usize> },
}

/// Arguments de ligne de commande analysés.
struct Cli {
    help: bool,
    config_path: Option<PathBuf>,
    command: Command,
}

impl Cli {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut help = false;
        let mut config_path: Option<PathBuf> = None;
        let mut positionals: Vec<String> = Vec::new();

        let mut it = args;
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "-c" | "--config" => {
                    let path = it
                        .next()
                        .ok_or_else(|| anyhow!("--config attend un chemin de fichier"))?;
                    config_path = Some(PathBuf::from(path));
                }
                s if s.starts_with("--config=") => {
                    config_path = Some(PathBuf::from(&s["--config=".len()..]));
                }
                s if s.starts_with('-') => bail!("option inconnue : {s}"),
                _ => positionals.push(arg),
            }
        }

        // L'aide court-circuite la validation de la commande.
        if help {
            return Ok(Cli {
                help,
                config_path,
                command: Command::Server { force_admin: false },
            });
        }

        let command = match positionals.first().map(String::as_str) {
            None => Command::Server { force_admin: false },
            Some("admin") => Command::Server { force_admin: true },
            Some("import") => {
                let path = positionals
                    .get(1)
                    .ok_or_else(|| anyhow!("usage : sesame import <fichier.json>"))?;
                Command::Import {
                    path: PathBuf::from(path),
                }
            }
            Some("preview") => {
                let count = positionals
                    .get(1)
                    .map(|s| s.parse::<usize>())
                    .transpose()
                    .map_err(|_| anyhow!("preview : le nombre de questions doit être un entier"))?;
                Command::Preview { count }
            }
            Some(other) => bail!("commande inconnue : {other}"),
        };

        Ok(Cli {
            help,
            config_path,
            command,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = match Cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("Erreur : {err}\n\n{HELP}");
            std::process::exit(2);
        }
    };

    if cli.help {
        println!("{HELP}");
        return Ok(());
    }

    let (cfg, cfg_path) = match &cli.config_path {
        Some(path) => (Config::load(path)?, path.clone()),
        None => Config::load_default()?,
    };
    println!("Configuration chargée depuis : {}", cfg_path.display());
    tracing::info!(config = %cfg_path.display(), "configuration chargée");

    match cli.command {
        Command::Import { path } => {
            let pool = db::init(&cfg.paths.database).await?;
            run_import(&pool, &path).await?;
        }
        Command::Preview { count } => {
            let pool = db::init(&cfg.paths.database).await?;
            run_preview(&pool, count).await?;
        }
        Command::Server { force_admin } => {
            let mode = if force_admin { "administration" } else { "portail" };
            tracing::info!("sesame démarré (mode {mode})");
            let pool = db::init(&cfg.paths.database).await?;
            run_server(cfg, pool, force_admin).await?;
        }
    }

    Ok(())
}

/// Démarre le serveur HTTP et ouvre le navigateur sur la bonne page.
///
/// `force_admin` : si vrai (commande `sesame admin`), on ouvre toujours
/// `/admin`. Sinon, on ouvre `/admin` quand aucun mot de passe administrateur
/// n'existe encore (pour le configurer), et `/` (l'examen) le reste du temps.
async fn run_server(cfg: Config, pool: SqlitePool, force_admin: bool) -> Result<()> {
    let host = cfg.server.host.clone();
    let port = cfg.server.port;

    let open_path = if force_admin || !auth::password_is_set(&pool).await? {
        "/admin"
    } else {
        "/"
    };

    let router = web::build_router(web::AppState { pool });

    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    let actual = listener.local_addr()?;

    // L'adresse de bind peut être 0.0.0.0 (toutes les interfaces) : ce n'est
    // pas une adresse navigable. Pour ouvrir le navigateur local, on remplace
    // une IP « non spécifiée » par le loopback.
    let browse_host = if actual.ip().is_unspecified() {
        if actual.is_ipv6() { "[::1]" } else { "127.0.0.1" }.to_string()
    } else {
        actual.ip().to_string()
    };
    let browse_url = format!("http://{browse_host}:{}{open_path}", actual.port());

    println!();
    println!("  ===========================================");
    println!("  sesame prêt");
    println!("  Local  : {browse_url}");
    println!("  Réseau : écoute sur http://{actual}");
    println!("  Ctrl+C pour arrêter.");
    println!("  ===========================================");
    println!();
    tracing::info!(%browse_url, listen = %actual, "server listening");

    if should_open_browser() {
        let url_for_open = browse_url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            open_browser(&url_for_open);
        });
    }

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Le confort du parent qui lance `sesame` à la main sur son bureau — jamais
/// une obligation. Le serveur est un SERVICE : c'est le kiosque qui affiche la
/// page, pas lui.
///
/// Sans `$DISPLAY` (console pure, service systemd, session à peine née), il
/// n'y a aucun bureau où ouvrir quoi que ce soit : `xdg-open` s'y perdrait et
/// se mettrait à proposer des navigateurs en mode texte. On n'essaie même pas.
fn should_open_browser() -> bool {
    if std::env::var_os("SESAME_NO_BROWSER").is_some() {
        return false;
    }
    if cfg!(target_os = "linux") {
        return std::env::var_os("DISPLAY").is_some()
            || std::env::var_os("WAYLAND_DISPLAY").is_some();
    }
    true
}

/// Ouvre le navigateur par défaut sur `url`, selon le système d'exploitation.
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(opener).arg(url).spawn() {
        Ok(_) => tracing::info!(%url, %opener, "navigateur ouvert"),
        Err(err) => tracing::warn!(
            ?err,
            %opener,
            %url,
            "ouverture automatique impossible — ouvre l'URL manuellement"
        ),
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(?err, "failed to listen for ctrl_c");
        return;
    }
    tracing::info!("shutdown signal received");
}

async fn run_import(pool: &SqlitePool, path: &Path) -> Result<()> {
    let report = importer::import_from_path(pool, path).await?;
    println!("=== Rapport d'importation ===");
    println!("Matières créées : {}", report.subjects_created);
    println!("Matières existantes (ignorées) : {}", report.subjects_skipped);
    println!("Questions importées : {}", report.questions_imported);
    if !report.questions_failed.is_empty() {
        println!("Questions échouées : {}", report.questions_failed.len());
        for (idx, err) in &report.questions_failed {
            println!("  - #{idx}: {err}");
        }
    }
    Ok(())
}

async fn run_preview(pool: &SqlitePool, n_override: Option<usize>) -> Result<()> {
    let child = policy::default_child(pool).await?;
    let n = n_override.unwrap_or(child.questions_per_test as usize);

    let questions =
        quiz::pick_questions(pool, n, child.difficulty_min, child.difficulty_max).await?;
    println!(
        "=== Aperçu du contrôle de {} ({} questions) ===",
        child.name,
        questions.len()
    );
    for (i, q) in questions.iter().enumerate() {
        println!("\n{}. [{}] {}", i + 1, q.kind, q.statement);
        for a in &q.answers {
            println!("   - ({}) {}", a.id, a.text);
        }
    }

    let mut submission: Submission = HashMap::new();
    for q in &questions {
        let correct: Vec<(i64,)> =
            sqlx::query_as("SELECT id FROM answers WHERE question_id = ? AND is_correct = 1")
                .bind(q.id)
                .fetch_all(pool)
                .await?;
        submission.insert(q.id, correct.into_iter().map(|(id,)| id).collect());
    }
    let result = quiz::grade(pool, &submission, child.pass_threshold_pct).await?;
    println!("\n=== Simulation: toutes les réponses correctes ===");
    println!(
        "Score : {}/{} ({:.1}%)",
        result.correct_count, result.total_count, result.score_pct
    );
    println!("Seuil de réussite : {:.1}%", result.threshold_pct);
    println!(
        "Résultat : {}",
        if result.passed { "RÉUSSI" } else { "ÉCHEC" }
    );

    let used = policy::consumed_today(pool, child.id).await? / 60;
    println!("\nTemps consommé aujourd'hui : {used} min");
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("sesame=info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
