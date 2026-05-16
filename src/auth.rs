use anyhow::{Context, Result, anyhow};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;
use sqlx::SqlitePool;

const PASSWORD_SETTING_KEY: &str = "admin_password_hash";
pub const SESSION_COOKIE_NAME: &str = "luanti_admin";
const DEFAULT_SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;

// ===== Password ==============================================================

pub fn hash_password(plain: &str) -> Result<String> {
    let salt = {
        let mut rng = rand::thread_rng();
        SaltString::generate(&mut rng)
    };
    let phc = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash: {e}"))?;
    Ok(phc.to_string())
}

pub fn verify_password(plain: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

pub async fn password_is_set(pool: &SqlitePool) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(PASSWORD_SETTING_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| !v.is_empty()).unwrap_or(false))
}

pub async fn read_password_hash(pool: &SqlitePool) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(PASSWORD_SETTING_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

pub async fn set_password(pool: &SqlitePool, plain: &str) -> Result<()> {
    let hash = hash_password(plain).context("hashing password")?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(PASSWORD_SETTING_KEY)
    .bind(&hash)
    .execute(pool)
    .await?;
    Ok(())
}

// ===== Sessions ==============================================================

pub async fn create_session(pool: &SqlitePool) -> Result<String> {
    let token = random_token();
    let now = chrono::Utc::now().timestamp();
    let expires = now + DEFAULT_SESSION_TTL_SECS;
    sqlx::query(
        "INSERT INTO admin_sessions (token, created_at, expires_at)
         VALUES (?, ?, ?)",
    )
    .bind(&token)
    .bind(now)
    .bind(expires)
    .execute(pool)
    .await?;
    Ok(token)
}

pub async fn validate_session(pool: &SqlitePool, token: &str) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM admin_sessions WHERE token = ? AND expires_at > ?")
            .bind(token)
            .bind(now)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn delete_session(pool: &SqlitePool, token: &str) -> Result<()> {
    sqlx::query("DELETE FROM admin_sessions WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn purge_expired_sessions(pool: &SqlitePool) -> Result<u64> {
    let now = chrono::Utc::now().timestamp();
    let r = sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
