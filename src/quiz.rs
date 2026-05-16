use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
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

/// What the child submitted: question_id → chosen answer_ids.
pub type Submission = HashMap<i64, Vec<i64>>;

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

pub async fn pick_questions(pool: &SqlitePool, n: usize) -> Result<Vec<QuizQuestion>> {
    if n == 0 {
        return Ok(Vec::new());
    }

    let rows: Vec<(i64, f64, i64)> = sqlx::query_as(
        "SELECT s.id, s.weight, COUNT(q.id)
         FROM subjects s
         LEFT JOIN questions q ON q.subject_id = s.id
         GROUP BY s.id",
    )
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
            "SELECT id FROM questions WHERE subject_id = ? ORDER BY RANDOM() LIMIT ?",
        )
        .bind(subject_id)
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
        let answers: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, text FROM answers WHERE question_id = ? ORDER BY RANDOM()",
        )
        .bind(qid)
        .fetch_all(pool)
        .await?;
        result.push(QuizQuestion {
            id: q.0,
            kind: q.1,
            statement: q.2,
            answers: answers
                .into_iter()
                .map(|(id, text)| QuizAnswer { id, text })
                .collect(),
        });
    }
    Ok(result)
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

// ===== Grader =====

pub async fn grade(pool: &SqlitePool, submission: &Submission) -> Result<GradedAttempt> {
    let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind("pass_threshold_pct")
        .fetch_one(pool)
        .await
        .context("reading pass_threshold_pct")?;
    let threshold_pct: f64 = raw.parse().context("pass_threshold_pct must be numeric")?;

    let mut graded = Vec::with_capacity(submission.len());

    for (&question_id, chosen_ids) in submission {
        let q: (i64, String, String, Option<String>) = sqlx::query_as(
            "SELECT id, kind, statement, explanation FROM questions WHERE id = ?",
        )
        .bind(question_id)
        .fetch_one(pool)
        .await?;

        let answer_rows: Vec<(i64, String, i64)> =
            sqlx::query_as("SELECT id, text, is_correct FROM answers WHERE question_id = ?")
                .bind(question_id)
                .fetch_all(pool)
                .await?;

        let chosen_set: HashSet<i64> = chosen_ids.iter().copied().collect();
        let correct_set: HashSet<i64> = answer_rows
            .iter()
            .filter(|(_, _, c)| *c == 1)
            .map(|(id, _, _)| *id)
            .collect();

        let correct = chosen_set == correct_set;

        let answers = answer_rows
            .into_iter()
            .map(|(id, text, is_corr)| GradedAnswer {
                answer_id: id,
                text,
                is_correct: is_corr == 1,
                was_chosen: chosen_set.contains(&id),
            })
            .collect();

        graded.push(GradedQuestion {
            question_id: q.0,
            kind: q.1,
            statement: q.2,
            explanation: q.3,
            answers,
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
}
