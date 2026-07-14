use std::path::Path;

use anyhow::{Context, Result, bail};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection, SqlitePool};

pub async fn init(database_path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data dir {}", parent.display()))?;
    }

    run_migrations(database_path).await?;

    let opts = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .with_context(|| format!("opening sqlite at {}", database_path.display()))?;

    tracing::info!(path = %database_path.display(), "base de données prête");
    Ok(pool)
}

/// Les migrations tournent sur une connexion DÉDIÉE, clés étrangères DÉSACTIVÉES.
///
/// SQLite ne sait pas modifier une contrainte CHECK : il faut reconstruire la
/// table (cf. 0006). Or `DROP TABLE questions` avec les clés étrangères ACTIVES
/// déclenche un DELETE implicite, qui fait CASCADER la suppression de toutes les
/// `answers` — la banque de questions entière, en silence.
///
/// Et ça ne peut se poser QU'ICI : `PRAGMA foreign_keys` est un no-op à
/// l'intérieur d'une transaction, et sqlx-sqlite enveloppe CHAQUE migration dans
/// une transaction (`sqlx-sqlite/src/migrate.rs` : `apply()` appelle `begin()`
/// sans condition — le `-- no-transaction` de sqlx-core n'est PAS honoré par ce
/// driver, contrairement à Postgres).
///
/// Le pool applicatif, lui, tourne clés étrangères actives : c'est le seul régime
/// sous lequel l'application écrit.
async fn run_migrations(database_path: &Path) -> Result<()> {
    let mut conn = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .foreign_keys(false)
        .connect()
        .await
        .with_context(|| format!("opening sqlite at {}", database_path.display()))?;

    sqlx::migrate!("./migrations")
        .run(&mut conn)
        .await
        .context("running migrations")?;

    // Filet de sécurité : les FK étant restées muettes pendant les migrations,
    // on vérifie explicitement qu'aucune n'est repartie pendante. Mieux vaut
    // refuser de démarrer qu'exploiter une base incohérente.
    let violations: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&mut conn)
            .await
            .context("foreign_key_check after migrations")?;

    conn.close().await.ok();

    if !violations.is_empty() {
        bail!(
            "migrations terminées sur une base incohérente : {} référence(s) pendante(s) ({})",
            violations.len(),
            violations
                .iter()
                .map(|(table, _, parent, _)| format!("{table} → {parent}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}
