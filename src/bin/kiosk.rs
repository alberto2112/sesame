//! `sesame-kiosk` — la porte.
//!
//! Tourne dans un serveur X **nu**, sans gestionnaire de fenêtres, AVANT que
//! le bureau n'existe. Il affiche le contrôle, attend que l'ordinateur soit
//! gagné, et **sort avec le code 0**. C'est tout son contrat avec le système :
//!
//! ```sh
//! sesame-kiosk || exit 1     # pas de code 0 → pas de bureau
//! exec startplasma-x11
//! ```
//!
//! ## Pourquoi il n'y a rien à fuir
//!
//! Alt+Tab, Alt+F4, « toujours au-dessus », minimiser : **rien de tout cela
//! n'est implémenté par X11**. C'est le gestionnaire de fenêtres qui le fait.
//! Il n'y en a pas ici : ces raccourcis ne sont pas désactivés, ils n'existent
//! pas. Personne à qui parler.
//!
//! ## Les deux pièges d'un X sans gestionnaire de fenêtres
//!
//! 1. **`fullscreen` ne marche pas.** Passer une fenêtre en plein écran est un
//!    *protocole* (`_NET_WM_STATE_FULLSCREEN`) adressé au gestionnaire de
//!    fenêtres. Sans lui, la demande tombe dans le vide et le navigateur
//!    s'ouvre à sa taille par défaut. On lui donne donc la géométrie
//!    explicitement : la taille de l'écran, en 0,0. Sans gestionnaire, la
//!    géométrie demandée est la géométrie obtenue.
//!
//! 2. **Personne n'attribue le focus clavier.** C'est aussi le travail du
//!    gestionnaire de fenêtres. Les navigateurs s'en sortent seuls quand ils
//!    sont l'unique client X ; si un jour ce n'est plus le cas, c'est ici
//!    qu'il faudra un `XSetInputFocus`.
//!
//! ## Le navigateur peut mourir, la porte non
//!
//! Si le navigateur se ferme (Ctrl+Q, plantage), on le **relance**. Le kiosque
//! ne rend la main que lorsque l'API dit que l'ordinateur est gagné.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use sesame::config::Config;

const HELP: &str = "\
sesame-kiosk — le contrôle, avant que le bureau n'existe

USAGE :
    sesame-kiosk [OPTIONS]

Affiche le contrôle en plein écran et attend que l'ordinateur soit gagné.
Sort avec le code 0 dès qu'une concession de temps est vivante — c'est le
signal, pour le script de session, qu'il peut lancer le bureau.

OPTIONS :
    -c, --config <fichier>   Utilise ce fichier de configuration.
    -h, --help               Affiche cette aide.

Le navigateur est choisi automatiquement, sauf si config.toml le précise :

    [kiosk]
    browser = \"chromium\"
";

/// Rythme des sondages de l'API. Assez court pour que l'enfant ne voie pas
/// l'attente, assez long pour ne pas marteler le serveur.
const POLL: Duration = Duration::from_secs(1);

/// Le serveur peut démarrer en même temps que nous : on lui laisse le temps.
const SERVER_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct GateResponse {
    unlocked: bool,
    #[serde(default)]
    child_name: Option<String>,
    #[serde(default)]
    remaining_secs: i64,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("sesame-kiosk : {err:#}");
        // Code ≠ 0 → le script de session s'arrête → pas de bureau. Un kiosque
        // qui plante ne doit JAMAIS ouvrir la porte.
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut config_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
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

    let (cfg, cfg_path) = match &config_path {
        Some(p) => (Config::load(p)?, p.clone()),
        None => Config::load_default()?,
    };
    eprintln!("sesame-kiosk : configuration {}", cfg_path.display());

    let base = cfg.local_url();
    wait_for_server(&base)?;

    // Déjà gagné ? Rien à afficher. Arrive après un plantage : on ne va pas
    // refaire passer un contrôle à un enfant qui a déjà payé.
    if gate(&base)?.unlocked {
        eprintln!("sesame-kiosk : ordinateur déjà débloqué — on passe la main");
        return Ok(());
    }

    let command = browser_command(&cfg)?;
    eprintln!("sesame-kiosk : navigateur → {}", command.join(" "));

    let mut browser = Browser::spawn(&command, &base)?;

    loop {
        match gate(&base) {
            Ok(g) if g.unlocked => {
                let who = g.child_name.unwrap_or_else(|| "quelqu'un".into());
                eprintln!(
                    "sesame-kiosk : {who} a gagné {} min — on ouvre le bureau",
                    (g.remaining_secs + 59) / 60
                );
                browser.kill();
                return Ok(());
            }
            Ok(_) => {}
            // Le serveur peut redémarrer sous nos pieds : on ne meurt pas pour
            // si peu. Mourir, ici, ce serait fermer la session de l'enfant.
            Err(err) => eprintln!("sesame-kiosk : API injoignable ({err}) — on réessaie"),
        }

        // Navigateur fermé (Ctrl+Q, plantage) ? On le relance. La porte reste
        // fermée tant que le contrôle n'est pas passé.
        if browser.has_exited() {
            eprintln!("sesame-kiosk : le navigateur s'est fermé — relance");
            browser = Browser::spawn(&command, &base)?;
        }

        sleep(POLL);
    }
}

// ===== API ==================================================================

fn gate(base: &str) -> Result<GateResponse> {
    let body: GateResponse = ureq::get(&format!("{base}/api/gate"))
        .timeout(Duration::from_secs(5))
        .call()?
        .into_json()?;
    Ok(body)
}

fn wait_for_server(base: &str) -> Result<()> {
    let deadline = Instant::now() + SERVER_WAIT;
    let mut last: Option<String> = None;
    while Instant::now() < deadline {
        match gate(base) {
            Ok(_) => return Ok(()),
            Err(err) => last = Some(err.to_string()),
        }
        sleep(Duration::from_millis(500));
    }
    bail!(
        "le serveur sesame ne répond pas sur {base} après {} s{}",
        SERVER_WAIT.as_secs(),
        last.map(|e| format!(" (dernière erreur : {e})")).unwrap_or_default()
    )
}

// ===== Le navigateur ========================================================

/// Tue le navigateur quand on s'en va — même en cas d'erreur. Sans ça, un
/// kiosque qui abandonne laisserait une fenêtre orpheline en plein écran.
struct Browser(Option<Child>);

impl Browser {
    fn spawn(command: &[String], base: &str) -> Result<Self> {
        let child = Command::new(&command[0])
            .args(&command[1..])
            .arg(base)
            .spawn()
            .with_context(|| format!("lancement du navigateur « {} »", command[0]))?;
        Ok(Self(Some(child)))
    }

    fn has_exited(&mut self) -> bool {
        match self.0.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            None => true,
        }
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Construit la ligne de commande du navigateur (l'URL est ajoutée à la fin
/// par [`Browser::spawn`]).
fn browser_command(cfg: &Config) -> Result<Vec<String>> {
    // Choix explicite du parent : on lui fait confiance, on n'ajoute rien.
    if let Some(custom) = &cfg.kiosk.browser {
        let parts: Vec<String> = custom.split_whitespace().map(String::from).collect();
        if parts.is_empty() {
            bail!("[kiosk] browser est vide dans la configuration");
        }
        return Ok(parts);
    }

    let (w, h) = screen_size().unwrap_or((1920, 1080));

    for (bin, flavour) in CANDIDATES {
        if which(bin).is_some() {
            return Ok(flavour.args(bin, w, h));
        }
    }

    bail!(
        "aucun navigateur trouvé. Installe-en un (par ex. « sudo pacman -S chromium ») \
         ou précise-le dans config.toml :\n\n    [kiosk]\n    browser = \"mon-navigateur --kiosk\""
    )
}

#[derive(Clone, Copy)]
enum Flavour {
    Chromium,
    Firefox,
}

impl Flavour {
    /// `--window-size` / `--width` : la géométrie explicite, indispensable sans
    /// gestionnaire de fenêtres (`--kiosk` seul ne suffit pas — voir l'en-tête).
    fn args(self, bin: &str, w: u16, h: u16) -> Vec<String> {
        let mut v = vec![bin.to_string()];
        match self {
            Flavour::Chromium => v.extend(
                [
                    "--kiosk",
                    "--incognito",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-features=TranslateUI",
                    "--window-position=0,0",
                    &format!("--window-size={w},{h}"),
                ]
                .map(String::from),
            ),
            Flavour::Firefox => v.extend(
                [
                    "--kiosk",
                    "--private-window",
                    "--width",
                    &w.to_string(),
                    "--height",
                    &h.to_string(),
                ]
                .map(String::from),
            ),
        }
        v
    }
}

const CANDIDATES: &[(&str, Flavour)] = &[
    ("chromium", Flavour::Chromium),
    ("chromium-browser", Flavour::Chromium),
    ("google-chrome-stable", Flavour::Chromium),
    ("brave", Flavour::Chromium),
    ("vivaldi-stable", Flavour::Chromium),
    ("firefox", Flavour::Firefox),
    ("firefox-esr", Flavour::Firefox),
];

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

// ===== La taille de l'écran =================================================

/// Sans gestionnaire de fenêtres, personne ne redimensionne rien : c'est à
/// nous de demander la bonne taille. On la lit directement sur le serveur X.
/// Hors X (poste de développement), on retombe sur les valeurs par défaut du
/// navigateur — sans intérêt pour un kiosque, mais ça ne l'empêche pas de
/// tourner.
fn screen_size() -> Option<(u16, u16)> {
    use x11rb::connection::Connection;

    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let screen = conn.setup().roots.get(screen_num)?;
    Some((screen.width_in_pixels, screen.height_in_pixels))
}
