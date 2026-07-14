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
    /// 1 (facile) à 5 (difficile). Absent = 3 : les fichiers existants restent
    /// valides et leurs questions visibles par tous les enfants.
    #[serde(default = "default_difficulty")]
    pub difficulty: i64,
}

fn default_difficulty() -> i64 {
    3
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
            "INSERT INTO questions (subject_id, kind, statement, explanation, difficulty, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(subject_id)
        .bind(&q.kind)
        .bind(q.statement.trim())
        .bind(q.explanation.as_ref().map(|s| s.trim().to_string()))
        .bind(q.difficulty)
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
    if !(1..=5).contains(&q.difficulty) {
        return Err(format!("difficulté {} hors de [1,5]", q.difficulty));
    }
    for (i, a) in q.answers.iter().enumerate() {
        if a.text.trim().is_empty() {
            return Err(format!("texte de la réponse #{} vide", i + 1));
        }
    }
    let correct = q.answers.iter().filter(|a| a.correct).count();
    let incorrect = q.answers.len() - correct;

    // 'exact'/'number' : la bonne réponse est STOCKÉE, une seule ligne. Elle n'est
    // jamais déduite de l'énoncé — voir migrations/0006.
    if crate::quiz::is_free_input(&q.kind) {
        if q.answers.len() != 1 {
            return Err(format!(
                "type '{}' exige exactement 1 réponse (la bonne), {} fournies",
                q.kind,
                q.answers.len()
            ));
        }
        if correct != 1 {
            return Err(format!(
                "type '{}' exige que sa réponse soit marquée correcte",
                q.kind
            ));
        }
        if q.kind == "number" && crate::quiz::parse_number(&q.answers[0].text).is_none() {
            return Err(format!(
                "type 'number' : la réponse '{}' n'est pas un nombre",
                q.answers[0].text.trim()
            ));
        }
        return Ok(());
    }

    if q.answers.len() < 2 {
        return Err(format!(
            "au moins 2 réponses requises, {} fournies",
            q.answers.len()
        ));
    }
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
                "type '{other}' invalide (attendu 'single', 'multi', 'exact' ou 'number')"
            ));
        }
    }
    Ok(())
}
