use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub paths: PathsConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct PathsConfig {
    pub game_binary: PathBuf,
    pub database: PathBuf,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        let mut cfg: Self = toml::from_str(&raw).context("parsing config TOML")?;
        cfg.paths.database = resolve_data_path(&cfg.paths.database);
        Ok(cfg)
    }

    /// Charge la configuration depuis le premier emplacement par défaut qui
    /// existe. Renvoie aussi le chemin retenu, pour que l'appelant puisse
    /// l'afficher (sinon impossible de savoir quel fichier est utilisé).
    pub fn load_default() -> Result<(Self, PathBuf)> {
        let candidates = default_config_candidates();
        for path in &candidates {
            if path.exists() {
                return Ok((Self::load(path)?, path.clone()));
            }
        }
        let tried = candidates
            .iter()
            .map(|p| format!("  - {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n");
        Err(anyhow!(
            "Aucun fichier de configuration trouvé. Chemins essayés :\n{tried}\n\
             Crée le fichier dans ~/.config/luanti-gate/config.toml, lance la commande \
             depuis le dossier du projet, ou passe --config <fichier>."
        ))
    }
}

fn default_config_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = dirs::config_dir() {
        out.push(dir.join("luanti-gate").join("config.toml"));
    }
    out.push(PathBuf::from("config.toml"));
    out
}

fn resolve_data_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let base = dirs::data_dir()
        .map(|d| d.join("luanti-gate"))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(p)
}
