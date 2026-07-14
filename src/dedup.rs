//! Détection et purge des questions en double.
//!
//! Le critère est l'ÉNONCÉ, et lui seul : deux questions au même énoncé sont un
//! doublon, quelles que soient leur matière, leur difficulté ou leurs réponses.
//! Dans chaque groupe on garde la PLUS RÉCENTE (id le plus haut) : c'est celle
//! qui vient du dernier import, donc la mieux tournée — meilleure explication,
//! plus d'options.
//!
//! Le piège est ailleurs. `attempt_questions.question_id` référence
//! `questions(id)` SANS `ON DELETE` : supprimer une question déjà tombée à un
//! contrôle échoue sur une violation de clé étrangère — et c'est justement la
//! plus ANCIENNE, celle qu'on veut supprimer, qui a le plus de chances d'avoir
//! un historique. On réaffecte donc son historique au survivant AVANT de la
//! supprimer. C'est licite parce que l'énoncé est IDENTIQUE : « à ce contrôle,
//! l'enfant a répondu à cette question-là » reste vrai, quelle que soit la ligne
//! qui la porte.
//!
//! `attempt_answers`, lui, n'est PAS touché : il stocke des instantanés de texte
//! sans clé étrangère (cf. 0002), et c'est la seule table que la page
//! d'historique lit vraiment. Elle continue de raconter ce qui s'est passé.

use std::collections::HashMap;

use anyhow::Result;
use sqlx::SqlitePool;

/// La clé de regroupement. Deux énoncés qui se réduisent à la même clé sont un
/// doublon.
///
/// Volontairement plus permissive que l'égalité stricte : une majuscule ou une
/// espace en trop ne fait pas une question différente pour l'enfant qui la lit.
/// D'où le regroupement EN RUST plutôt qu'un `GROUP BY LOWER(statement)` :
/// `LOWER()` en SQLite est purement ASCII, il ne descend pas « Été » sur « été »
/// — inutilisable sur une banque de questions en français.
pub fn dedup_key(statement: &str) -> String {
    statement
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Un groupe de doublons : le survivant, et ce qui tombe.
pub struct DuplicateGroup {
    pub statement: String,
    pub keep_id: i64,
    pub victims: Vec<Victim>,
}

pub struct Victim {
    pub id: i64,
    pub subject_name: String,
    /// Nombre de contrôles où cette question est tombée. > 0 = son historique
    /// sera réaffecté au survivant.
    pub history_count: i64,
}

#[derive(Debug, Default)]
pub struct PurgeReport {
    pub groups: usize,
    pub deleted: usize,
    /// Lignes d'historique réaffectées au survivant.
    pub repointed: u64,
    /// Lignes d'historique jetées : le contrôle contenait DÉJÀ le survivant —
    /// les deux doublons y étaient tombés ensemble. Fusionner ferait doublon sur
    /// la clé primaire (attempt_id, question_id) ; on garde celle du survivant.
    pub merged: u64,
}

/// Les groupes de doublons, du plus gros au plus petit.
pub async fn find_duplicates(pool: &SqlitePool) -> Result<Vec<DuplicateGroup>> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT q.id, q.statement, s.name
         FROM questions q JOIN subjects s ON s.id = q.subject_id
         ORDER BY q.id",
    )
    .fetch_all(pool)
    .await?;

    let mut by_key: HashMap<String, Vec<(i64, String, String)>> = HashMap::new();
    for row in rows {
        by_key.entry(dedup_key(&row.1)).or_default().push(row);
    }

    let mut groups = Vec::new();
    for (_, mut members) in by_key {
        if members.len() < 2 {
            continue;
        }
        members.sort_by_key(|(id, _, _)| *id);
        // Le survivant : l'id le plus haut, donc le dernier après tri.
        let (keep_id, statement, _) = members.pop().expect("len >= 2");

        let mut victims = Vec::with_capacity(members.len());
        for (id, _, subject_name) in members {
            let (history_count,): (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM attempt_questions WHERE question_id = ?")
                    .bind(id)
                    .fetch_one(pool)
                    .await?;
            victims.push(Victim {
                id,
                subject_name,
                history_count,
            });
        }
        groups.push(DuplicateGroup {
            statement,
            keep_id,
            victims,
        });
    }

    groups.sort_by(|a, b| {
        b.victims
            .len()
            .cmp(&a.victims.len())
            .then(a.statement.cmp(&b.statement))
    });
    Ok(groups)
}

/// Supprime tous les doublons. Tout ou rien : une seule transaction.
pub async fn purge(pool: &SqlitePool) -> Result<PurgeReport> {
    let groups = find_duplicates(pool).await?;
    let mut report = PurgeReport {
        groups: groups.len(),
        ..Default::default()
    };

    let mut tx = pool.begin().await?;
    for group in &groups {
        for victim in &group.victims {
            // 1. Le contrôle contenait-il DÉJÀ le survivant ? Alors la ligne du
            //    doublon ne peut pas être réaffectée — (attempt_id, question_id)
            //    est la clé primaire. On la jette : celle du survivant reste, et
            //    l'enfant avait de toute façon répondu deux fois au même énoncé.
            let merged = sqlx::query(
                "DELETE FROM attempt_questions
                  WHERE question_id = ?
                    AND attempt_id IN (SELECT attempt_id FROM attempt_questions
                                        WHERE question_id = ?)",
            )
            .bind(victim.id)
            .bind(group.keep_id)
            .execute(&mut *tx)
            .await?;
            report.merged += merged.rows_affected();

            // 2. Le reste de l'historique passe au survivant : sans ça, le DELETE
            //    ci-dessous échoue sur la clé étrangère (pas de ON DELETE).
            let moved = sqlx::query(
                "UPDATE attempt_questions SET question_id = ? WHERE question_id = ?",
            )
            .bind(group.keep_id)
            .bind(victim.id)
            .execute(&mut *tx)
            .await?;
            report.repointed += moved.rows_affected();

            // 3. La question part, ses `answers` avec elle (ON DELETE CASCADE).
            //    `attempt_answers` n'a pas de FK : ses instantanés survivent.
            sqlx::query("DELETE FROM questions WHERE id = ?")
                .bind(victim.id)
                .execute(&mut *tx)
                .await?;
            report.deleted += 1;
        }
    }
    tx.commit().await?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_ignores_case_and_spacing() {
        assert_eq!(
            dedup_key("  Combien   de pattes a une araignée ? "),
            dedup_key("combien de pattes a une araignée ?")
        );
    }

    #[test]
    fn key_is_unicode_aware() {
        // Ce que le LOWER() de SQLite ne sait PAS faire.
        assert_eq!(dedup_key("Quelle SAISON ?"), dedup_key("quelle saison ?"));
        assert_eq!(dedup_key("ÉTÉ ou hiver ?"), dedup_key("été ou hiver ?"));
    }

    #[test]
    fn key_keeps_genuinely_different_statements_apart() {
        assert_ne!(dedup_key("2 + 2 = ?"), dedup_key("2 + 3 = ?"));
        // Les accents restent significatifs : ce ne sont pas les mêmes mots.
        assert_ne!(dedup_key("ou ?"), dedup_key("où ?"));
    }
}
