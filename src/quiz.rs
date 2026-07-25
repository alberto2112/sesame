use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rand::seq::SliceRandom;
use sqlx::SqlitePool;

use crate::grid::{self, Grid, Toggle};

// ===== Types exposed to routes/templates =====

#[derive(Debug, Clone)]
pub struct QuizQuestion {
    pub id: i64,
    pub kind: String,
    pub statement: String,
    pub answers: Vec<QuizAnswer>,
    /// 'grid_cells' / 'grid_lines' : le modèle à reproduire et la grille vierge.
    /// `None` pour tous les autres types.
    pub grid: Option<GridPrompt>,
}

#[derive(Debug, Clone)]
pub struct QuizAnswer {
    pub id: i64,
    pub text: String,
}

/// Tout ce dont le gabarit a besoin pour poser l'exercice de reproduction —
/// déjà rendu, déjà calculé. Askama ne fait pas de géométrie ; Rust si.
#[derive(Debug, Clone)]
pub struct GridPrompt {
    pub w: i64,
    pub h: i64,
    /// Rapport largeur/hauteur du cadre, marge du dessin comprise — c'est lui
    /// qui fait coïncider les cases à cocher avec le quadrillage dessiné.
    pub aspect: String,
    /// SVG du modèle, à gauche.
    pub model_svg: String,
    /// SVG du quadrillage nu, sous les cases à cocher, à droite.
    pub blank_svg: String,
    /// Les zones cliquables de la grille vierge.
    pub toggles: Vec<Toggle>,
    /// `true` pour 'grid_cells' — le gabarit s'en sert pour la consigne et
    /// pour l'habillage des marques (une case pleine, ou un trait).
    pub is_cells: bool,
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
    /// 'grid_cells' / 'grid_lines' : les cases ou segments marqués, jeton par
    /// jeton, dans l'ordre où le navigateur les a envoyés. La mise en forme
    /// canonique n'a pas lieu ici mais dans `grid` : un seul endroit décide de
    /// ce que « ce dessin » veut dire.
    Grid(Vec<String>),
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
    /// Modèle et dessin de l'enfant, côte à côte, pour la page de correction.
    /// `None` hors des types grille.
    pub grid: Option<GridReview>,
    pub correct: bool,
}

/// La correction d'une reproduction : le modèle, et le dessin de l'enfant
/// colorié en trois couleurs (juste / en trop / oublié). Un « raté » sans le
/// dessin sous les yeux n'apprend rien à personne.
#[derive(Debug, Clone)]
pub struct GridReview {
    pub w: i64,
    pub h: i64,
    pub aspect: String,
    pub model_svg: String,
    pub given_svg: String,
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
///
/// Le poids et l'activation de chaque matière sont PROPRES à l'enfant
/// (`child_subject_weights`). La ligne enfant×matière fait foi ; à défaut —
/// matière ajoutée après l'enfant — on retombe (`COALESCE`) sur la valeur par
/// défaut globale de `subjects`.
pub async fn pick_questions(
    pool: &SqlitePool,
    child_id: i64,
    n: usize,
    diff_min: i64,
    diff_max: i64,
) -> Result<Vec<QuizQuestion>> {
    if n == 0 {
        return Ok(Vec::new());
    }

    let rows: Vec<(i64, f64, i64)> = sqlx::query_as(
        "SELECT s.id, COALESCE(csw.weight, s.weight), COUNT(q.id)
         FROM subjects s
         LEFT JOIN child_subject_weights csw
                ON csw.subject_id = s.id AND csw.child_id = ?
         LEFT JOIN questions q ON q.subject_id = s.id
                              AND q.difficulty BETWEEN ? AND ?
         WHERE COALESCE(csw.enabled, s.enabled) = 1
         GROUP BY s.id",
    )
    .bind(child_id)
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

        // Trois régimes, et un seul mot d'ordre : ce qui doit rester secret ne
        // quitte pas le serveur.
        //   - QCM       : les options partent, forcément — l'enfant y choisit.
        //   - écrit     : rien ne part. Un Ctrl+U lirait la réponse.
        //   - grille    : le modèle part, et c'est TOUT L'EXERCICE. Voir la
        //                 figure ne dispense pas de savoir la recopier.
        let (answers, grid) = if is_grid(&q.1) {
            // Une figure illisible ne donne PAS une question sans grille : elle
            // donne une carte sans le moindre champ, et `quiz.js` refuse de
            // valider un contrôle dont une question n'est pas répondue. L'enfant
            // se retrouverait bloqué, incapable de terminer — la porte fermée
            // par un bug d'adulte. On retire donc la question du contrôle : il
            // en comptera une de moins, et c'est très bien.
            let Some(prompt) = load_grid_prompt(pool, qid, &q.1).await? else {
                continue;
            };
            (Vec::new(), Some(prompt))
        } else if answer_is_secret(&q.1) {
            (Vec::new(), None)
        } else {
            let options = sqlx::query_as::<_, (i64, String)>(
                "SELECT id, text FROM answers WHERE question_id = ? ORDER BY RANDOM()",
            )
            .bind(qid)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(id, text)| QuizAnswer { id, text })
            .collect();
            (options, None)
        };

        result.push(QuizQuestion {
            id: q.0,
            kind: q.1,
            statement: q.2,
            answers,
            grid,
        });
    }
    Ok(result)
}

/// Charge la figure modèle et prépare la grille vierge.
///
/// Une payload illisible ne fait pas tomber le contrôle : la question part sans
/// grille et le gabarit n'affiche rien à reproduire. Elle sera comptée fausse,
/// ce qui est mauvais — mais planter la page, c'est refuser l'ordinateur à
/// l'enfant pour une question mal saisie par un adulte.
async fn load_grid_prompt(
    pool: &SqlitePool,
    question_id: i64,
    kind: &str,
) -> Result<Option<GridPrompt>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT text FROM answers WHERE question_id = ? AND is_correct = 1 LIMIT 1",
    )
    .bind(question_id)
    .fetch_optional(pool)
    .await?;

    let Some((payload,)) = row else {
        tracing::warn!(question_id, "question grille sans figure en base");
        return Ok(None);
    };

    let model = match Grid::parse_as(kind, &payload) {
        Ok(g) => g,
        Err(err) => {
            tracing::warn!(question_id, %err, "figure illisible");
            return Ok(None);
        }
    };

    Ok(Some(GridPrompt {
        w: model.w,
        h: model.h,
        aspect: grid::frame_aspect(model.w, model.h),
        model_svg: model.svg(),
        blank_svg: model.svg_blank(),
        toggles: grid::toggles(kind, model.w, model.h),
        is_cells: kind == grid::KIND_CELLS,
    }))
}

/// Types dont la réponse s'écrit au clavier, par opposition aux types à choix.
pub fn is_free_input(kind: &str) -> bool {
    matches!(kind, "exact" | "number")
}

/// Types où l'enfant reproduit une figure sur une grille.
pub fn is_grid(kind: &str) -> bool {
    grid::is_grid_kind(kind)
}

/// Types dont l'unique ligne d'`answers` porte LA bonne réponse, et non des
/// options à proposer. C'est la règle de validation partagée par l'importeur et
/// le panel admin : exactement une réponse, marquée correcte.
pub fn stores_single_answer(kind: &str) -> bool {
    is_free_input(kind) || is_grid(kind)
}

/// « Cette réponse doit-elle rester sur le serveur ? »
///
/// À ne pas confondre avec [`is_free_input`], même si les deux ont longtemps
/// donné le même verdict — jusqu'aux grilles. Une réponse écrite est secrète :
/// l'envoyer, c'est la donner. Un modèle à reproduire est PUBLIC par nature :
/// c'est l'énoncé lui-même. Confondre les deux axes, c'était soit dévoiler une
/// réponse, soit rendre l'exercice impossible.
pub fn answer_is_secret(kind: &str) -> bool {
    is_free_input(kind)
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
        let (correct, given_text, grid_review) = match (kind, given) {
            // Les grilles passent en premier et attrapent TOUTES les formes de
            // réponse, y compris l'absence de réponse. Une question sautée doit
            // rester fausse — ça va de soi — mais elle doit aussi produire sa
            // correction : l'enfant qui n'a rien dessiné a justement besoin de
            // voir le modèle qu'il n'a pas recopié.
            (k, g) if is_grid(k) => {
                let tokens: &[String] = match g {
                    Given::Grid(t) => t,
                    _ => &[],
                };
                let model = answer_rows
                    .iter()
                    .find(|(_, _, c)| *c == 1)
                    .and_then(|(_, text, _)| Grid::parse_as(k, text).ok());
                match model {
                    Some(model) => {
                        let drawn = Grid::from_tokens(k, model.w, model.h, tokens);
                        // 100 % ou rien : une seule marque en trop ou en moins,
                        // et le dessin n'est pas le même. C'est la règle de
                        // l'exercice, pas une sévérité qu'on ajoute.
                        let correct = drawn == model;
                        let review = GridReview {
                            w: model.w,
                            h: model.h,
                            aspect: grid::frame_aspect(model.w, model.h),
                            model_svg: model.svg(),
                            given_svg: model.svg_review(Some(&drawn)),
                        };
                        let serialized = if drawn.is_empty() {
                            None
                        } else {
                            Some(drawn.serialize())
                        };
                        (correct, serialized, Some(review))
                    }
                    // Figure absente ou illisible : la question est mal saisie et
                    // impossible à réussir. Même politique qu'ailleurs — on ne
                    // devine pas ce que l'adulte a voulu dessiner.
                    None => (false, None, None),
                }
            }
            ("single" | "multi", Given::Choices(ids)) => {
                let chosen: HashSet<i64> = ids.iter().copied().collect();
                let expected: HashSet<i64> = answer_rows
                    .iter()
                    .filter(|(_, _, c)| *c == 1)
                    .map(|(id, _, _)| *id)
                    .collect();
                (chosen == expected, None, None)
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
                (correct, typed, None)
            }
            _ => (false, None, None),
        };

        // Pour les types à réponse unique stockée (écrite ou dessinée),
        // `answer_rows` tient LA bonne réponse : `was_chosen` y vaut « l'enfant
        // est tombé dessus ». La page de correction affiche donc « c'était la
        // bonne réponse » exactement comme pour un QCM.
        let chosen_ids: HashSet<i64> = match given {
            Given::Choices(ids) => ids.iter().copied().collect(),
            Given::Text(_) | Given::Grid(_) => HashSet::new(),
        };
        let answers = answer_rows
            .into_iter()
            .map(|(id, text, is_corr)| {
                let is_correct = is_corr == 1;
                GradedAnswer {
                    answer_id: id,
                    text,
                    is_correct,
                    was_chosen: if stores_single_answer(kind) {
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
            grid: grid_review,
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
