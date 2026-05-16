use std::path::Path;

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

pub async fn init(database_path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data dir {}", parent.display()))?;
    }

    let opts = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .with_context(|| format!("opening sqlite at {}", database_path.display()))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;

    tracing::info!(path = %database_path.display(), "base de données prête");
    Ok(pool)
}
