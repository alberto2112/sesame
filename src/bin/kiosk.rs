//! `sesame-kiosk` — la porte.
//!
//! Tourne **dans `cage`**, un compositeur kiosque, AVANT que le bureau
//! n'existe. Il affiche le contrôle, attend que l'ordinateur soit gagné, et
//! **sort avec le code 0**. C'est tout son contrat avec le système :
//!
//! ```sh
//! cage -- sesame-kiosk
//! sesame-kiosk --check || exit 1     # pas débloqué → pas de bureau
//! exec startplasma-wayland
//! ```
//!
//! ## Pourquoi il n'y a rien à fuir
//!
//! Alt+Tab, Alt+F4, « toujours au-dessus », minimiser : **rien de tout cela
//! n'est implémenté par Wayland**. C'est le compositeur qui le fait — et cage
//! n'en fait rien du tout. Il n'a qu'une politique : une application, plein
//! écran. Ces raccourcis ne sont pas désactivés, ils n'existent pas. Personne
//! à qui parler.
//!
//! ## Ce que cage nous a rendu
//!
//! Cette porte a vécu jusqu'à Plasma 5 dans un serveur X **nu**, sans
//! gestionnaire de fenêtres. Le plein écran y était un *protocole*
//! (`_NET_WM_STATE_FULLSCREEN`) adressé à un gestionnaire… qui n'existait pas :
//! `--kiosk` tombait dans le vide, et il fallait lire la taille de l'écran sur
//! le serveur X pour la passer à la main au navigateur.
//!
//! Sous cage il y a un vrai compositeur, qui honore le plein écran. `--kiosk`
//! suffit. Toute la géométrie explicite a disparu — et `x11rb` avec elle.
//!
//! ## Les deux navigateurs ont chacun leur clé Wayland
//!
//! Sans elle, ils tentent X11, ne trouvent personne (cage n'apporte pas
//! forcément XWayland) et meurent :
//!
//! - Chromium : `--ozone-platform=wayland`
//! - Firefox : `MOZ_ENABLE_WAYLAND=1` dans l'environnement
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
        --check              Demande une seule fois si l'ordinateur est
                             débloqué, sans rien afficher. Sort 0 (oui) ou
                             1 (non, ou serveur muet). C'est le verdict que
                             sesame-session interroge après cage.
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
    let mut check_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "--check" => check_only = true,
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

    // `--check` : le verdict, rien d'autre. `sesame-session` le pose APRÈS
    // cage, parce que le code de sortie de cage ne dit rien de fiable sur ce
    // qui s'est passé dedans. La seule source de vérité, c'est
    // `policy::evaluate` derrière /api/gate — alors on la lui demande.
    //
    // Serveur muet, réponse illisible, ordinateur verrouillé : tout tombe du
    // même côté, le code 1. Une serrure doit échouer FERMÉE.
    if check_only {
        wait_for_server(&base)?;
        let g = gate(&base)?;
        if !g.unlocked {
            bail!("ordinateur verrouillé — aucune concession de temps vivante");
        }
        let who = g.child_name.unwrap_or_else(|| "quelqu'un".into());
        eprintln!(
            "sesame-kiosk : débloqué pour {who} ({} min) — on ouvre le bureau",
            (g.remaining_secs + 59) / 60
        );
        return Ok(());
    }

    wait_for_server(&base)?;

    // Déjà gagné ? Rien à afficher. Arrive après un plantage : on ne va pas
    // refaire passer un contrôle à un enfant qui a déjà payé.
    if gate(&base)?.unlocked {
        eprintln!("sesame-kiosk : ordinateur déjà débloqué — on passe la main");
        return Ok(());
    }

    let command = browser_command(&cfg)?;
    eprintln!("sesame-kiosk : navigateur → {command}");

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
    fn spawn(command: &BrowserCmd, base: &str) -> Result<Self> {
        let child = Command::new(&command.argv[0])
            .args(&command.argv[1..])
            .arg(base)
            .envs(command.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .spawn()
            .with_context(|| format!("lancement du navigateur « {} »", command.argv[0]))?;
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

/// Ce qu'il faut pour lancer le navigateur : sa ligne de commande, et
/// l'environnement sans lequel il ne trouverait pas Wayland.
struct BrowserCmd {
    argv: Vec<String>,
    env: Vec<(String, String)>,
}

impl std::fmt::Display for BrowserCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (k, v) in &self.env {
            write!(f, "{k}={v} ")?;
        }
        f.write_str(&self.argv.join(" "))
    }
}

/// Construit la commande du navigateur (l'URL est ajoutée à la fin par
/// [`Browser::spawn`]).
fn browser_command(cfg: &Config) -> Result<BrowserCmd> {
    // Choix explicite du parent : on lui fait confiance, on n'ajoute rien. À
    // lui de mettre la clé Wayland de son navigateur — elle est documentée
    // dans config.toml.
    if let Some(custom) = &cfg.kiosk.browser {
        let argv: Vec<String> = custom.split_whitespace().map(String::from).collect();
        if argv.is_empty() {
            bail!("[kiosk] browser est vide dans la configuration");
        }
        return Ok(BrowserCmd { argv, env: vec![] });
    }

    for (bin, flavour) in CANDIDATES {
        if which(bin).is_some() {
            return Ok(flavour.command(bin));
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
    /// Plus de géométrie explicite : sous cage il y a un vrai compositeur, et
    /// `--kiosk` est enfin honoré. Ne reste que la clé Wayland — sans elle, le
    /// navigateur part chercher un serveur X et meurt. Les versions récentes la
    /// devinent seules ; on la donne quand même, parce qu'un kiosque qui
    /// affiche un écran noir, c'est un enfant enfermé dehors.
    fn command(self, bin: &str) -> BrowserCmd {
        let mut argv = vec![bin.to_string()];
        let mut env = vec![];
        match self {
            Flavour::Chromium => argv.extend(
                [
                    "--kiosk",
                    "--ozone-platform=wayland",
                    "--incognito",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-features=TranslateUI",
                ]
                .map(String::from),
            ),
            Flavour::Firefox => {
                argv.extend(["--kiosk", "--private-window"].map(String::from));
                env.push(("MOZ_ENABLE_WAYLAND".to_string(), "1".to_string()));
            }
        }
        BrowserCmd { argv, env }
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
