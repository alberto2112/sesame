use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rand::seq::SliceRandom;
use sqlx::SqlitePool;

// ===== Types exposed to routes/templates =====

#[derive(Debug, Clone)]
pub struct QuizQuestion {
    pub id: i64,
    pub kind: String,
    pub statement: String,
    pub answers: Vec<QuizAnswer>,
}

#[derive(Debug, Clone)]
pub struct QuizAnswer {
    pub id: i64,
    pub text: String,
}

/// Ce que l'enfant a donné pour UNE question. Le type de la question décide
/// laquelle des deux formes est valable ; c'est `grade` qui tranche, à partir du
/// `kind` en base — jamais à partir de ce que le formulaire prétend être.
#[derive(Debug, Clone)]
pub enum Given {
    /// 'single' / 'multi' : les identifiants des réponses cochées.
    Choices(Vec<i64>),
    /// 'exact' / 'number' : le texte saisi.
    Text(String),
}

impl Default for Given {
    /// Une question sautée : aucune case cochée, aucun texte. Toujours fausse.
    fn default() -> Self {
        Given::Choices(Vec::new())
    }
}

/// What the child submitted: question_id → what they gave.
pub type Submission = HashMap<i64, Given>;

#[derive(Debug, Clone)]
pub struct GradedAttempt {
    pub questions: Vec<GradedQuestion>,
    pub correct_count: usize,
    pub total_count: usize,
    pub score_pct: f64,
    pub threshold_pct: f64,
    pub passed: bool,
}

#[derive(Debug, Clone)]
pub struct GradedQuestion {
    pub question_id: i64,
    pub kind: String,
    pub statement: String,
    pub explanation: Option<String>,
    pub answers: Vec<GradedAnswer>,
    /// Ce que l'enfant a écrit ('exact'/'number' seulement). None pour les types
    /// à choix : l'information y vit déjà dans `was_chosen`.
    pub given_text: Option<String>,
    pub correct: bool,
}

#[derive(Debug, Clone)]
pub struct GradedAnswer {
    pub answer_id: i64,
    pub text: String,
    pub is_correct: bool,
    pub was_chosen: bool,
}

// ===== Selector =====

/// `diff_min..=diff_max` : plage de difficulté de l'ENFANT. Le filtre
/// s'applique aussi au comptage par matière, pour que la répartition
/// proportionnelle se fasse sur les questions réellement disponibles
/// pour cet enfant, pas sur le total.
pub async fn pick_questions(
    pool: &SqlitePool,
    n: usize,
    diff_min: i64,
    diff_max: i64,
) -> Result<Vec<QuizQuestion>> {
    if n == 0 {
        return Ok(Vec::new());
    }

    let rows: Vec<(i64, f64, i64)> = sqlx::query_as(
        "SELECT s.id, s.weight, COUNT(q.id)
         FROM subjects s
         LEFT JOIN questions q ON q.subject_id = s.id
                              AND q.difficulty BETWEEN ? AND ?
         WHERE s.enabled = 1
         GROUP BY s.id",
    )
    .bind(diff_min)
    .bind(diff_max)
    .fetch_all(pool)
    .await?;

    let subjects: Vec<(i64, f64, usize)> = rows
        .into_iter()
        .map(|(id, w, c)| (id, w, c as usize))
        .filter(|(_, w, av)| *w > 0.0 && *av > 0)
        .collect();

    let allocations = distribute(&subjects, n);

    let mut question_ids: Vec<i64> = Vec::new();
    for (subject_id, count) in &allocations {
        if *count == 0 {
            continue;
        }
        let ids: Vec<(i64,)> = sqlx::query_as(
            "SELECT id FROM questions
             WHERE subject_id = ? AND difficulty BETWEEN ? AND ?
             ORDER BY RANDOM() LIMIT ?",
        )
        .bind(subject_id)
        .bind(diff_min)
        .bind(diff_max)
        .bind(*count as i64)
        .fetch_all(pool)
        .await?;
        question_ids.extend(ids.into_iter().map(|(id,)| id));
    }

    {
        let mut rng = rand::thread_rng();
        question_ids.shuffle(&mut rng);
    }

    let mut result = Vec::with_capacity(question_ids.len());
    for qid in question_ids {
        let q: (i64, String, String) =
            sqlx::query_as("SELECT id, kind, statement FROM questions WHERE id = ?")
                .bind(qid)
                .fetch_one(pool)
                .await?;

        // Pour 'exact'/'number', la table `answers` ne contient PAS des options :
        // elle contient LA bonne réponse. La joindre au rendu, c'est l'écrire dans
        // le HTML — un Ctrl+U et l'enfant lit le résultat. On ne la charge donc
        // même pas : la correction se fait côté serveur, dans `grade`.
        let answers = if is_free_input(&q.1) {
            Vec::new()
        } else {
            sqlx::query_as::<_, (i64, String)>(
                "SELECT id, text FROM answers WHERE question_id = ? ORDER BY RANDOM()",
            )
            .bind(qid)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(id, text)| QuizAnswer { id, text })
            .collect()
        };

        result.push(QuizQuestion {
            id: q.0,
            kind: q.1,
            statement: q.2,
            answers,
        });
    }
    Ok(result)
}

/// Types dont la réponse s'écrit au clavier, par opposition aux types à choix.
pub fn is_free_input(kind: &str) -> bool {
    matches!(kind, "exact" | "number")
}

/// Pure allocation algorithm (Hamilton/largest remainder + iterative cap).
/// Input: (subject_id, weight, available_questions)
/// Output: (subject_id, n_questions_to_pick)
fn distribute(subjects: &[(i64, f64, usize)], n: usize) -> Vec<(i64, usize)> {
    let mut active: Vec<(i64, f64, usize)> = subjects.iter().copied().collect();
    let mut result: HashMap<i64, usize> = HashMap::new();
    let mut remaining = n;

    loop {
        if active.is_empty() || remaining == 0 {
            break;
        }
        let total_w: f64 = active.iter().map(|(_, w, _)| w).sum();
        if total_w <= 0.0 {
            break;
        }

        let mut alloc: HashMap<i64, usize> = HashMap::new();
        let mut fracs: Vec<(i64, f64)> = Vec::with_capacity(active.len());
        let mut sum_floor = 0usize;

        for (id, w, _) in &active {
            let target = remaining as f64 * (w / total_w);
            let floor = target.floor() as usize;
            alloc.insert(*id, floor);
            fracs.push((*id, target - floor as f64));
            sum_floor += floor;
        }

        let leftover = remaining.saturating_sub(sum_floor);
        fracs.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        for (id, _) in fracs.iter().take(leftover) {
            *alloc.get_mut(id).expect("id from same loop") += 1;
        }

        let mut overflowed: Vec<(i64, usize)> = Vec::new();
        for (id, _, av) in &active {
            let t = alloc[id];
            if t > *av {
                overflowed.push((*id, *av));
            }
        }

        if overflowed.is_empty() {
            for (id, c) in alloc {
                *result.entry(id).or_insert(0) += c;
            }
            break;
        }

        for (id, av) in &overflowed {
            *result.entry(*id).or_insert(0) += av;
            remaining = remaining.saturating_sub(*av);
            active.retain(|(sid, _, _)| sid != id);
        }
    }

    result.into_iter().collect()
}

// ===== Comparaison des réponses écrites =====

/// 'exact' : extrémités rognées, casse ignorée. `to_lowercase` est Unicode —
/// « ÉTÉ » vaut « été ». Les ACCENTS, eux, comptent : « ou » n'est pas « où »,
/// et sur une question d'orthographe c'est précisément ce qu'on évalue.
fn text_matches(given: &str, expected: &str) -> bool {
    given.trim().to_lowercase() == expected.trim().to_lowercase()
}

/// 'number' : on compare des NOMBRES, pas des chaînes. Un enfant qui écrit
/// « 08 », « +8 » ou « 8,0 » a donné la bonne réponse — le recaler sur la forme
/// serait lui refuser l'ordinateur pour un zéro devant.
fn number_matches(given: &str, expected: &str) -> bool {
    match (parse_number(given), parse_number(expected)) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-9,
        // Réponse attendue non numérique : la question est mal saisie. On retombe
        // sur la comparaison texte au lieu de punir l'enfant d'une faute d'adulte.
        _ => text_matches(given, expected),
    }
}

/// Virgule décimale française acceptée, espaces (y compris insécables des
/// milliers) ignorés.
///
/// Publique à dessein : l'importeur et le panel admin valident « est-ce un
/// nombre ? » avec CETTE fonction. Deux définitions divergentes de « nombre »
/// (l'une à l'écriture, l'autre à la correction) laisseraient passer une
/// question impossible à réussir — « 2,5 » acceptée à l'import, jamais reconnue
/// à la correction.
pub fn parse_number(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse().ok()
}

// ===== Grader =====

/// `threshold_pct` vient de l'enfant, pas des réglages globaux : un enfant de
/// 6 ans et un de 10 ans ne passent pas la même barre.
pub async fn grade(
    pool: &SqlitePool,
    submission: &Submission,
    threshold_pct: f64,
) -> Result<GradedAttempt> {
    let mut graded = Vec::with_capacity(submission.len());

    for (&question_id, given) in submission {
        let q: (i64, String, String, Option<String>) = sqlx::query_as(
            "SELECT id, kind, statement, explanation FROM questions WHERE id = ?",
        )
        .bind(question_id)
        .fetch_one(pool)
        .await?;
        let kind = q.1.as_str();

        let answer_rows: Vec<(i64, String, i64)> =
            sqlx::query_as("SELECT id, text, is_correct FROM answers WHERE question_id = ?")
                .bind(question_id)
                .fetch_all(pool)
                .await?;

        // La forme de la réponse est dictée par le `kind` EN BASE, pas par celle
        // que le formulaire a envoyée : un couple incohérent (du texte pour un
        // QCM, des cases pour une question écrite) est un formulaire trafiqué, et
        // se solde par « faux ». On ne fait jamais confiance au client.
        let (correct, given_text) = match (kind, given) {
            ("single" | "multi", Given::Choices(ids)) => {
                let chosen: HashSet<i64> = ids.iter().copied().collect();
                let expected: HashSet<i64> = answer_rows
                    .iter()
                    .filter(|(_, _, c)| *c == 1)
                    .map(|(id, _, _)| *id)
                    .collect();
                (chosen == expected, None)
            }
            ("exact" | "number", Given::Text(typed)) => {
                let expected = answer_rows.iter().find(|(_, _, c)| *c == 1);
                let correct = match expected {
                    Some((_, text, _)) if kind == "number" => number_matches(typed, text),
                    Some((_, text, _)) => text_matches(typed, text),
                    // Question sans bonne réponse en base : impossible de la
                    // réussir. Elle ne devrait pas exister (importeur + admin la
                    // refusent), mais on ne devine pas.
                    None => false,
                };
                // Rien de saisi = question sautée : `None`, pas `Some("")`. La page
                // de correction et l'historique n'ont pas à distinguer les deux, et
                // ça évite d'avoir à tester le vide dans un template Askama.
                let typed = if typed.trim().is_empty() {
                    None
                } else {
                    Some(typed.clone())
                };
                (correct, typed)
            }
            _ => (false, None),
        };

        // Pour 'exact'/'number', `answer_rows` tient l'unique bonne réponse :
        // `was_chosen` y vaut « l'enfant est tombé dessus ». La page de correction
        // affiche donc « c'était la bonne réponse » exactement comme pour un QCM.
        let chosen_ids: HashSet<i64> = match given {
            Given::Choices(ids) => ids.iter().copied().collect(),
            Given::Text(_) => HashSet::new(),
        };
        let answers = answer_rows
            .into_iter()
            .map(|(id, text, is_corr)| {
                let is_correct = is_corr == 1;
                GradedAnswer {
                    answer_id: id,
                    text,
                    is_correct,
                    was_chosen: if is_free_input(kind) {
                        is_correct && correct
                    } else {
                        chosen_ids.contains(&id)
                    },
                }
            })
            .collect();

        graded.push(GradedQuestion {
            question_id: q.0,
            kind: q.1,
            statement: q.2,
            explanation: q.3,
            answers,
            given_text,
            correct,
        });
    }

    let total_count = graded.len();
    let correct_count = graded.iter().filter(|q| q.correct).count();
    let score_pct = if total_count == 0 {
        0.0
    } else {
        (correct_count as f64 / total_count as f64) * 100.0
    };
    let passed = score_pct >= threshold_pct;

    Ok(GradedAttempt {
        questions: graded,
        correct_count,
        total_count,
        score_pct,
        threshold_pct,
        passed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(v: Vec<(i64, usize)>) -> HashMap<i64, usize> {
        v.into_iter().collect()
    }

    #[test]
    fn equal_weights_distributes_evenly() {
        let subjects = vec![
            (1, 1.0, 100),
            (2, 1.0, 100),
            (3, 1.0, 100),
            (4, 1.0, 100),
        ];
        let m = collect(distribute(&subjects, 10));
        assert_eq!(m.values().sum::<usize>(), 10);
        for c in m.values() {
            assert!(*c == 2 || *c == 3, "value {c} outside expected 2..=3");
        }
    }

    #[test]
    fn skewed_weights_respect_proportion() {
        let subjects = vec![(1, 0.8, 100), (2, 0.1, 100), (3, 0.1, 100)];
        let m = collect(distribute(&subjects, 10));
        assert_eq!(m[&1], 8);
        assert_eq!(m[&2] + m[&3], 2);
        assert_eq!(m.values().sum::<usize>(), 10);
    }

    #[test]
    fn overflow_caps_and_redistributes() {
        let subjects = vec![(1, 0.5, 2), (2, 0.25, 100), (3, 0.25, 100)];
        let m = collect(distribute(&subjects, 10));
        assert_eq!(m[&1], 2);
        assert_eq!(m.values().sum::<usize>(), 10);
    }

    #[test]
    fn n_exceeds_total_returns_all_available() {
        let subjects = vec![(1, 1.0, 3), (2, 1.0, 5)];
        let m = collect(distribute(&subjects, 100));
        assert_eq!(m[&1], 3);
        assert_eq!(m[&2], 5);
        assert_eq!(m.values().sum::<usize>(), 8);
    }

    #[test]
    fn zero_n_returns_empty() {
        let subjects = vec![(1, 1.0, 5)];
        assert!(distribute(&subjects, 0).is_empty());
    }

    // ===== Comparaisons =====
    // Le faux négatif est le pire bug de cette application : l'enfant a juste,
    // et la machine reste verrouillée. Ces cas sont là pour ça.

    #[test]
    fn exact_ignores_case_and_surrounding_space() {
        assert!(text_matches("  Chien ", "chien"));
        assert!(text_matches("ÉTÉ", "été"));
    }

    #[test]
    fn exact_keeps_accents_significant() {
        // Sur une question d'orthographe, c'est justement ce qu'on évalue.
        assert!(!text_matches("ou", "où"));
    }

    #[test]
    fn number_ignores_formatting() {
        for given in ["8", "08", "+8", " 8 ", "8,0", "8.0"] {
            assert!(number_matches(given, "8"), "« {given} » devrait valoir 8");
        }
    }

    #[test]
    fn number_handles_negatives_and_decimals() {
        assert!(number_matches("-12", "-12"));
        assert!(number_matches("2,5", "2.5"));
        assert!(!number_matches("9", "8"));
        assert!(!number_matches("-8", "8"));
    }

    #[test]
    fn number_rejects_empty_or_garbage() {
        assert!(!number_matches("", "8"));
        assert!(!number_matches("huit", "8"));
    }

    #[test]
    fn number_falls_back_to_text_when_expected_is_not_a_number() {
        // Question mal saisie : on compare comme du texte plutôt que de recaler
        // l'enfant pour une faute qui n'est pas la sienne.
        assert!(number_matches("Huit", "huit"));
    }
}
