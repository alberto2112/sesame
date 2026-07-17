//! `sesame-timer` — l'horloge.
//!
//! Tourne dans la session de l'enfant, derrière le bureau. Il fait DEUX choses
//! et rien d'autre :
//!
//! 1. Il dit au serveur combien de secondes viennent de s'écouler.
//! 2. Quand il n'en reste plus, il prévient, puis il ferme la porte.
//!
//! ## Pourquoi une horloge MONOTONE
//!
//! Le temps écoulé est mesuré avec [`Instant`] — l'horloge monotone du noyau,
//! celle qui ne recule jamais. Le serveur ne fait qu'additionner ce qu'on lui
//! envoie ; il ne regarde JAMAIS l'heure de la machine pour savoir combien de
//! temps l'enfant a joué.
//!
//! Sans ça, un enfant qui apprend à reculer l'horloge du système se fabrique
//! des minutes à volonté. Avec ça, changer l'heure ne change rien du tout.
//!
//! Effet de bord voulu : sous Linux, `CLOCK_MONOTONIC` **ne compte pas** le
//! temps passé en veille. Une machine en veille n'est pas une machine qu'on
//! utilise — ces minutes-là ne doivent pas être facturées.

use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use sesame::config::Config;

const HELP: &str = "\
sesame-timer — l'horloge de la session

USAGE :
    sesame-timer [OPTIONS]

Décompte le temps accordé, prévient l'enfant avant la fin, puis applique le
mode de verrouillage choisi par le parent (réglage « lock_mode » de /admin).

OPTIONS :
    -c, --config <fichier>   Utilise ce fichier de configuration.
    -h, --help               Affiche cette aide.
";

/// Rythme normal : on ne martèle pas le serveur pour rien.
const TICK: Duration = Duration::from_secs(15);

/// Rythme de la dernière ligne droite, pour que le compte à rebours soit juste.
const TICK_FINAL: Duration = Duration::from_secs(2);

/// Rien à décompter (personne n'a de temps) : on regarde de loin en loin.
const TICK_IDLE: Duration = Duration::from_secs(20);

/// Paliers d'avertissement, en secondes restantes. L'enfant DOIT pouvoir
/// sauvegarder son dessin : on ne coupe jamais sans prévenir.
const WARNINGS: &[i64] = &[300, 120, 60, 30, 10];

/// Sursis après le dernier avertissement, avant de couper pour de bon.
const GRACE: Duration = Duration::from_secs(10);

/// Le temps qu'on laisse à Plasma pour s'en aller de lui-même après qu'on le
/// lui a demandé. Normalement on ne voit jamais la fin de cette attente : la
/// session meurt, et le minuteur avec elle.
const PLASMA_EXIT_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Deserialize)]
struct Gate {
    unlocked: bool,
    #[serde(default)]
    child_id: Option<i64>,
    #[serde(default)]
    child_name: Option<String>,
    #[serde(default)]
    remaining_secs: i64,
    #[serde(default)]
    lock_mode: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("sesame-timer : {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cfg = parse_args()?;
    let base = cfg.local_url();
    eprintln!("sesame-timer : démarré, serveur {base}");

    // Point de référence monotone. Tout le décompte part d'ici.
    let mut last_beat = Instant::now();
    let mut warned_below: Option<i64> = None;

    loop {
        let gate = match gate(&base) {
            Ok(g) => g,
            // Le serveur peut redémarrer sous nos pieds. On ne coupe pas la
            // session de l'enfant pour une requête ratée : dans le doute, on
            // le laisse jouer. C'est le sens de la faute.
            Err(err) => {
                eprintln!("sesame-timer : API injoignable ({err}) — on réessaie");
                last_beat = Instant::now();
                sleep(TICK_IDLE);
                continue;
            }
        };

        if !gate.unlocked {
            // La porte s'est refermée PENDANT que nous tournons. Le minuteur
            // ne vit QUE dans la session de l'enfant (sesame-session le lance
            // APRÈS l'ouverture du bureau) : « personne n'a de temps » ne peut
            // pas vouloir dire « on patiente à la porte » — ça veut dire « le
            // bureau tourne sans droit ». Couvre-feu atteint, concession
            // révoquée depuis /admin : peu importe la raison, on ferme.
            //
            // Sans cette branche, le couvre-feu ne coupait JAMAIS : la fin de
            // fenêtre horaire arrive par l'horloge murale, pas par nos
            // battements, donc c'est CE gate() — pas le heartbeat — qui la
            // voit en premier, et l'ancienne branche « idle » dormait dessus
            // pendant que l'enfant gardait le bureau ouvert des heures.
            eprintln!("sesame-timer : la porte est fermée — on clôt la session");
            notify(
                "Temps écoulé",
                "C'est fini pour cette fois. On ferme…",
            );
            sleep(GRACE);
            return expire(&gate.lock_mode);
        }

        let Some(child_id) = gate.child_id else {
            sleep(TICK_IDLE);
            continue;
        };

        // Le cœur : des secondes MONOTONES, pas une heure de l'horloge murale.
        let elapsed = last_beat.elapsed().as_secs() as i64;
        let remaining = if elapsed >= 1 {
            last_beat = Instant::now();
            match heartbeat(&base, child_id, elapsed) {
                Ok(g) => g.remaining_secs,
                Err(err) => {
                    eprintln!("sesame-timer : battement perdu ({err})");
                    gate.remaining_secs
                }
            }
        } else {
            gate.remaining_secs
        };

        let who = gate.child_name.unwrap_or_else(|| "l'enfant".into());

        if remaining <= 0 {
            eprintln!("sesame-timer : temps écoulé pour {who}");
            notify(
                "Temps écoulé",
                &format!("{who}, c'est fini pour cette fois. On ferme…"),
            );
            sleep(GRACE);
            return expire(&gate.lock_mode);
        }

        // Un seul avertissement par palier franchi : on prévient, on ne harcèle
        // pas.
        //
        // `.rev()` n'est PAS un détail. WARNINGS est décroissant : sans lui,
        // `find` renverrait le premier palier satisfait — le plus LÂCHE (300 s
        // est vrai dès qu'il reste moins de 5 minutes). L'enfant serait
        // prévenu une seule fois, à 5 minutes, puis coupé sans compte à
        // rebours. On veut le palier le plus SERRÉ : celui qu'on vient de
        // franchir.
        if let Some(&palier) = WARNINGS.iter().rev().find(|&&w| remaining <= w) {
            if warned_below != Some(palier) {
                warned_below = Some(palier);
                let msg = countdown_message(remaining);
                eprintln!("sesame-timer : avertissement — {msg}");
                notify("Il te reste peu de temps", &format!("{who}, {msg}"));
            }
        }

        sleep(if remaining <= 60 { TICK_FINAL } else { TICK });
    }
}

fn countdown_message(remaining: i64) -> String {
    if remaining >= 60 {
        let min = (remaining + 59) / 60;
        format!("il te reste {min} minute{}. Pense à sauvegarder !", plural(min))
    } else {
        format!("il te reste {remaining} secondes. Sauvegarde vite !")
    }
}

fn plural(n: i64) -> &'static str {
    if n > 1 { "s" } else { "" }
}

// ===== La coupure ===========================================================

fn expire(lock_mode: &str) -> Result<()> {
    match lock_mode {
        // La fenêtre de verrouillage (phase 7) a été ANNULÉE : sous Wayland, un
        // client ne peut ni capter le clavier ni recouvrir l'écran. Fermer la
        // session ramène à la porte — même résultat, et ça marche partout.
        "overlay" => {
            eprintln!(
                "sesame-timer : mode « overlay » abandonné (impossible sous Wayland) — \
                 on ferme la session"
            );
            logout()
        }
        _ => logout(),
    }
}

/// Ferme la session. Le script de session rend alors la main à SDDM, qui
/// relance… le kiosque. La boucle est bouclée : pour revenir, il faut repasser
/// un contrôle.
///
/// ## L'ordre n'est PAS un détail — il a coûté une soirée
///
/// SDDM lit le code de sortie de la session. **Tout ce qui n'est pas 0, il
/// l'appelle « Process crashed », il COUPE l'autologin, et il s'arrête là.**
/// C'est une protection anti-boucle, et elle a raison d'exister.
///
/// `loginctl terminate-session` ne demande rien à personne : il TUE. Plasma
/// meurt avec un code d'erreur, `sesame-session` le propage, SDDM croit à un
/// plantage — et l'enfant ne revient JAMAIS à la porte. Il reste dehors jusqu'à
/// ce qu'un adulte redémarre la machine. C'est exactement ce qui est arrivé le
/// soir du premier contrôle de Maël.
///
/// Alors on demande d'abord POLIMENT. Plasma s'en va de lui-même et rend 0 ;
/// SDDM voit une fin de session normale et ramène l'enfant à la porte. Le
/// marteau reste, mais en DERNIER : mieux vaut une session tuée qu'une session
/// qui refuse de se fermer après l'heure.
fn logout() -> Result<()> {
    // `org.kde.Shutdown.logout()` : sans argument, sans confirmation. Vérifié
    // sur la machine — c'est l'interface de Plasma 6 (l'ancienne, sur
    // `/KSMServer` avec trois entiers, était celle de Plasma 5).
    //
    // Le binaire, lui, a changé de nom avec Qt6 : `qdbus6`. `qdbus` peut être
    // celui de Qt5, ou ne pas exister du tout. On essaie les deux.
    for qdbus in ["qdbus6", "qdbus"] {
        if run_ok(
            qdbus,
            &["org.kde.Shutdown", "/Shutdown", "org.kde.Shutdown.logout"],
        ) {
            eprintln!("sesame-timer : déconnexion demandée à Plasma ({qdbus})");

            // Plasma part de lui-même. On l'attend — et normalement on ne se
            // réveille pas de ce sommeil : la session meurt, et nous avec elle.
            // Si on en sort, c'est que Plasma n'est pas parti.
            sleep(PLASMA_EXIT_WAIT);
            eprintln!("sesame-timer : Plasma n'est pas parti — on force");
            break;
        }
    }

    // Le marteau. SDDM criera au plantage et coupera l'autologin — mais un
    // ordinateur qui reste ouvert après l'heure serait pire qu'une porte close.
    let session = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "self".into());
    if run_ok("loginctl", &["terminate-session", &session]) {
        eprintln!("sesame-timer : session {session} fermée de force (loginctl)");
        return Ok(());
    }

    bail!(
        "impossible de fermer la session : ni Plasma ni loginctl n'ont répondu. \
         Le temps est écoulé mais l'ordinateur reste ouvert — préviens le parent."
    )
}

fn run_ok(bin: &str, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Avertissement visible. `-t 0` = la notification reste à l'écran : un enfant
/// de 6 ans ne lit pas un message qui disparaît en trois secondes.
fn notify(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["-u", "critical", "-t", "0", "-a", "Sésame", title, body])
        .status();
}

// ===== API ==================================================================

fn gate(base: &str) -> Result<Gate> {
    Ok(ureq::get(&format!("{base}/api/gate"))
        .timeout(Duration::from_secs(5))
        .call()?
        .into_json()?)
}

fn heartbeat(base: &str, child_id: i64, secs: i64) -> Result<Gate> {
    // Le serveur répond avec la décision à jour ; on lui refait confiance pour
    // le temps restant plutôt que de le calculer nous-mêmes. Une seule source
    // de vérité, y compris ici.
    ureq::post(&format!("{base}/api/heartbeat"))
        .timeout(Duration::from_secs(5))
        .send_json(ureq::json!({ "child_id": child_id, "secs": secs }))?;

    gate(base)
}

// ===== Arguments ============================================================

fn parse_args() -> Result<Config> {
    let mut config_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            "-c" | "--config" => {
                config_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--config attend un chemin de fichier"))?,
                ));
            }
            s if s.starts_with("--config=") => {
                config_path = Some(PathBuf::from(&s["--config=".len()..]));
            }
            s => bail!("option inconnue : {s}\n\n{HELP}"),
        }
    }

    match config_path {
        Some(p) => Config::load(&p),
        None => Config::load_default().map(|(c, _)| c),
    }
}
