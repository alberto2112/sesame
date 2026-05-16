use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::SqlitePool;

#[derive(Debug, Deserialize)]
pub struct ImportFile {
    #[serde(default)]
    pub subjects: Vec<ImportSubject>,
    pub questions: Vec<ImportQuestion>,
}

#[derive(Debug, Deserialize)]
pub struct ImportSubject {
    pub name: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
pub struct ImportQuestion {
    pub subject: String,
    pub kind: String,
    pub statement: String,
    pub answers: Vec<ImportAnswer>,
    #[serde(default)]
    pub explanation: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportAnswer {
    pub text: String,
    pub correct: bool,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub subjects_created: usize,
    pub subjects_skipped: usize,
    pub questions_imported: usize,
    pub questions_failed: Vec<(usize, String)>,
}

pub async fn import_from_path(pool: &SqlitePool, path: &Path) -> Result<ImportReport> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let file: ImportFile = serde_json::from_str(&raw).context("parsing JSON")?;
    import(pool, file).await
}

pub async fn import(pool: &SqlitePool, file: ImportFile) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let mut tx = pool.begin().await?;

    for s in &file.subjects {
        let name = s.name.trim();
        if name.is_empty() || s.weight <= 0.0 {
            continue;
        }
        let res = sqlx::query("INSERT OR IGNORE INTO subjects (name, weight) VALUES (?, ?)")
            .bind(name)
            .bind(s.weight)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() > 0 {
            report.subjects_created += 1;
        } else {
            report.subjects_skipped += 1;
        }
    }

    for (idx, q) in file.questions.iter().enumerate() {
        if let Err(e) = validate_question(q) {
            report.questions_failed.push((idx, e));
            continue;
        }

        let subject_row: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM subjects WHERE name = ?")
                .bind(q.subject.trim())
                .fetch_optional(&mut *tx)
                .await?;

        let subject_id = match subject_row {
            Some((id,)) => id,
            None => {
                report.questions_failed.push((
                    idx,
                    format!(
                        "matière '{}' non déclarée (ni dans le fichier, ni dans la base)",
                        q.subject
                    ),
                ));
                continue;
            }
        };

        let now = chrono::Utc::now().timestamp();
        let inserted: (i64,) = sqlx::query_as(
            "INSERT INTO questions (subject_id, kind, statement, explanation, created_at)
             VALUES (?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(subject_id)
        .bind(&q.kind)
        .bind(q.statement.trim())
        .bind(q.explanation.as_ref().map(|s| s.trim().to_string()))
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;

        for a in &q.answers {
            sqlx::query("INSERT INTO answers (question_id, text, is_correct) VALUES (?, ?, ?)")
                .bind(inserted.0)
                .bind(a.text.trim())
                .bind(if a.correct { 1 } else { 0 })
                .execute(&mut *tx)
                .await?;
        }
        report.questions_imported += 1;
    }

    tx.commit().await?;
    Ok(report)
}

fn validate_question(q: &ImportQuestion) -> Result<(), String> {
    if q.statement.trim().is_empty() {
        return Err("énoncé vide".into());
    }
    if q.answers.len() < 2 {
        return Err(format!(
            "au moins 2 réponses requises, {} fournies",
            q.answers.len()
        ));
    }
    for (i, a) in q.answers.iter().enumerate() {
        if a.text.trim().is_empty() {
            return Err(format!("texte de la réponse #{} vide", i + 1));
        }
    }
    let correct = q.answers.iter().filter(|a| a.correct).count();
    let incorrect = q.answers.len() - correct;
    match q.kind.as_str() {
        "single" => {
            if correct != 1 {
                return Err(format!(
                    "type 'single' exige exactement 1 réponse correcte, {correct} trouvées"
                ));
            }
        }
        "multi" => {
            if correct < 1 {
                return Err("type 'multi' exige au moins 1 réponse correcte".into());
            }
            if incorrect < 1 {
                return Err("type 'multi' exige au moins 1 réponse incorrecte".into());
            }
        }
        other => {
            return Err(format!(
                "type '{other}' invalide (attendu 'single' ou 'multi')"
            ));
        }
    }
    Ok(())
}
