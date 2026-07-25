//! Grilles à reproduire — les types de question 'grid_cells' et 'grid_lines'.
//!
//! L'exercice : un modèle à gauche, une grille vierge à droite, et l'enfant
//! recopie. Deux variantes, celles qu'il fait à l'école —
//!   * `grid_cells` : des CASES sont coloriées et forment une figure ;
//!   * `grid_lines` : des SEGMENTS relient les points du treillis (horizontaux,
//!     verticaux et diagonaux à 45°).
//!
//! ## La forme canonique, et pourquoi tout le module tourne autour d'elle
//!
//! Deux dessins IDENTIQUES peuvent s'écrire de mille façons : cases cliquées
//! dans un autre ordre, segment tracé de droite à gauche, ou un long trait d'un
//! seul geste là où un autre enfant en a fait trois courts. Comparer les
//! chaînes de caractères recalerait un dessin juste — et le faux négatif est le
//! pire bug de cette application : l'enfant a raison, et la machine reste
//! verrouillée.
//!
//! D'où la règle unique : tout entre par [`Grid::parse`] ou
//! [`Grid::from_tokens`], et en ressort sous forme d'ENSEMBLE trié —
//!   * cases  → un ensemble de `(ligne, colonne)` ;
//!   * lignes → un ensemble d'ARÊTES UNITAIRES, extrémités ordonnées. Un
//!     segment long est DÉCOUPÉ en arêtes d'une case. « (0,0)→(0,3) » et
//!     « (0,0)→(0,1) + (0,1)→(0,2) + (0,2)→(0,3) » deviennent le même objet.
//!
//! Corriger, c'est alors comparer deux ensembles avec `==`. Rien d'autre.
//! Même discipline que `quiz::parse_number` : UNE seule définition de « ce
//! dessin », partagée par l'importeur, le panel admin et la correction.

use std::collections::BTreeSet;
use std::f64::consts::SQRT_2;
use std::fmt::Write as _;

pub const KIND_CELLS: &str = "grid_cells";
pub const KIND_LINES: &str = "grid_lines";

/// Côté minimal/maximal d'une grille. La borne haute n'est pas pédagogique
/// (l'école va jusqu'à 8×8), elle est défensive : au-delà, une payload trafiquée
/// ferait rendre des dizaines de milliers de cases dans le DOM.
pub const MIN_SIDE: i64 = 2;
pub const MAX_SIDE: i64 = 12;

pub fn is_grid_kind(kind: &str) -> bool {
    kind == KIND_CELLS || kind == KIND_LINES
}

/// `(ligne, colonne)`. Une CASE vit dans `0..h` × `0..w` ; un SOMMET du treillis
/// vit dans `0..=h` × `0..=w` — il y a un point de plus que de cases, dans les
/// deux sens.
pub type Point = (i64, i64);

/// Arête d'UNE case de long, extrémités toujours triées : `a <= b`. C'est cet
/// ordre imposé qui fait que tracer de droite à gauche donne le même objet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub a: Point,
    pub b: Point,
}

impl Edge {
    pub fn new(a: Point, b: Point) -> Self {
        if a <= b { Edge { a, b } } else { Edge { a: b, b: a } }
    }

    pub fn token(&self) -> String {
        format!("{},{}-{},{}", self.a.0, self.a.1, self.b.0, self.b.1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Figure {
    Cells(BTreeSet<Point>),
    Edges(BTreeSet<Edge>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// Nombre de colonnes de cases.
    pub w: i64,
    /// Nombre de lignes de cases.
    pub h: i64,
    pub figure: Figure,
}

// ===== Lecture ==============================================================

impl Grid {
    /// Format : `LxH:c=r,c;r,c…` (cases) ou `LxH:e=r,c-r,c;…` (segments).
    ///
    /// Autodescriptif à dessein : la taille voyage AVEC la figure. Une colonne
    /// `width`/`height` de plus dans `questions` aurait été un tiroir vide pour
    /// les quatre autres types de question — et la taille aurait pu s'y
    /// désynchroniser de la payload, ce qui n'a aucun sens ici.
    pub fn parse(raw: &str) -> Result<Grid, String> {
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if cleaned.is_empty() {
            return Err("figure vide : attendu « 8x8:c=… » ou « 8x8:e=… »".into());
        }

        let (dims, rest) = cleaned
            .split_once(':')
            .ok_or_else(|| format!("« {raw} » : il manque le « : » après la taille"))?;

        let (w, h) = dims
            .split_once('x')
            .ok_or_else(|| format!("taille « {dims} » illisible (attendu « 8x8 »)"))?;
        let w: i64 = w
            .parse()
            .map_err(|_| format!("largeur « {w} » n'est pas un nombre"))?;
        let h: i64 = h
            .parse()
            .map_err(|_| format!("hauteur « {h} » n'est pas un nombre"))?;
        for (label, side) in [("largeur", w), ("hauteur", h)] {
            if !(MIN_SIDE..=MAX_SIDE).contains(&side) {
                return Err(format!(
                    "{label} {side} hors de [{MIN_SIDE},{MAX_SIDE}]"
                ));
            }
        }

        let (marker, body) = rest
            .split_once('=')
            .ok_or_else(|| format!("« {rest} » : attendu « c=… » ou « e=… »"))?;
        let tokens: Vec<&str> = body.split(';').filter(|t| !t.is_empty()).collect();

        let figure = match marker {
            "c" => {
                let mut cells = BTreeSet::new();
                for t in tokens {
                    cells.insert(parse_cell(t, w, h)?);
                }
                Figure::Cells(cells)
            }
            "e" => {
                let mut edges = BTreeSet::new();
                for t in tokens {
                    for e in parse_segment(t, w, h)? {
                        edges.insert(e);
                    }
                }
                Figure::Edges(edges)
            }
            other => {
                return Err(format!(
                    "marqueur « {other} » inconnu (attendu « c » pour les cases, « e » pour les segments)"
                ));
            }
        };

        Ok(Grid { w, h, figure })
    }

    /// Comme [`Grid::parse`], mais vérifie en plus que la figure correspond bien
    /// au type déclaré. Une payload de cases rangée sous 'grid_lines' donnerait
    /// une question impossible : la grille afficherait des points à relier, et
    /// la correction attendrait des cases.
    pub fn parse_as(kind: &str, raw: &str) -> Result<Grid, String> {
        let g = Grid::parse(raw)?;
        if g.kind() != kind {
            return Err(format!(
                "type '{kind}' déclaré, mais la figure est une figure de type '{}'",
                g.kind()
            ));
        }
        Ok(g)
    }

    /// Contrôle d'un MODÈLE (celui que l'adulte saisit), par opposition au
    /// dessin de l'enfant : une figure vide donnerait une question qu'on réussit
    /// sans rien faire.
    pub fn validate_model(kind: &str, raw: &str) -> Result<Grid, String> {
        let g = Grid::parse_as(kind, raw)?;
        if g.is_empty() {
            return Err("le modèle est vide : il n'y a rien à reproduire".into());
        }
        Ok(g)
    }

    /// Ce que l'enfant a coché, jeton par jeton — un par case ou par segment.
    ///
    /// Tolérant par choix : un jeton illisible est ignoré, pas fatal. Et il n'y
    /// a rien à protéger ici, contrairement aux autres types : le modèle est
    /// AFFICHÉ. Qui saurait fabriquer une requête saurait aussi bien recopier
    /// le dessin qu'il a sous les yeux — la seule chose que la sévérité
    /// achèterait, c'est le risque de recaler un enfant pour un bug à nous.
    pub fn from_tokens(kind: &str, w: i64, h: i64, tokens: &[String]) -> Grid {
        let figure = if kind == KIND_CELLS {
            let mut cells = BTreeSet::new();
            for t in tokens {
                if let Ok(cell) = parse_cell(t.trim(), w, h) {
                    cells.insert(cell);
                }
            }
            Figure::Cells(cells)
        } else {
            let mut edges = BTreeSet::new();
            for t in tokens {
                if let Ok(list) = parse_segment(t.trim(), w, h) {
                    edges.extend(list);
                }
            }
            Figure::Edges(edges)
        };
        Grid { w, h, figure }
    }

    /// Grille de mêmes dimensions et même nature, sans aucune marque : le fond
    /// de l'éditeur, et le « rien dessiné » d'une question sautée.
    pub fn blank_like(&self) -> Grid {
        Grid {
            w: self.w,
            h: self.h,
            figure: match self.figure {
                Figure::Cells(_) => Figure::Cells(BTreeSet::new()),
                Figure::Edges(_) => Figure::Edges(BTreeSet::new()),
            },
        }
    }

    pub fn kind(&self) -> &'static str {
        match self.figure {
            Figure::Cells(_) => KIND_CELLS,
            Figure::Edges(_) => KIND_LINES,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.marks() == 0
    }

    /// Nombre de marques — sert à doser la difficulté et à décrire la figure.
    pub fn marks(&self) -> usize {
        match &self.figure {
            Figure::Cells(c) => c.len(),
            Figure::Edges(e) => e.len(),
        }
    }

    /// Écriture canonique : c'est ce qui part en base et dans les instantanés
    /// d'historique. `parse(serialize(g)) == g`, toujours.
    pub fn serialize(&self) -> String {
        let mut out = format!("{}x{}:", self.w, self.h);
        match &self.figure {
            Figure::Cells(cells) => {
                out.push_str("c=");
                let body: Vec<String> =
                    cells.iter().map(|(r, c)| format!("{r},{c}")).collect();
                out.push_str(&body.join(";"));
            }
            Figure::Edges(edges) => {
                out.push_str("e=");
                let body: Vec<String> = edges.iter().map(Edge::token).collect();
                out.push_str(&body.join(";"));
            }
        }
        out
    }

    /// Résumé lisible pour le panel admin — « grille 8×8, 12 segments ».
    pub fn describe(&self) -> String {
        let (n, what) = match &self.figure {
            Figure::Cells(c) => (c.len(), if c.len() > 1 { "cases" } else { "case" }),
            Figure::Edges(e) => (e.len(), if e.len() > 1 { "segments" } else { "segment" }),
        };
        format!("grille {}×{}, {} {}", self.w, self.h, n, what)
    }
}

fn parse_cell(token: &str, w: i64, h: i64) -> Result<Point, String> {
    let (r, c) = token
        .split_once(',')
        .ok_or_else(|| format!("case « {token} » illisible (attendu « ligne,colonne »)"))?;
    let r: i64 = r
        .parse()
        .map_err(|_| format!("case « {token} » : ligne illisible"))?;
    let c: i64 = c
        .parse()
        .map_err(|_| format!("case « {token} » : colonne illisible"))?;
    if !(0..h).contains(&r) || !(0..w).contains(&c) {
        return Err(format!(
            "case « {token} » hors de la grille {w}×{h}"
        ));
    }
    Ok((r, c))
}

fn parse_vertex(token: &str, w: i64, h: i64) -> Result<Point, String> {
    let (r, c) = token
        .split_once(',')
        .ok_or_else(|| format!("point « {token} » illisible (attendu « ligne,colonne »)"))?;
    let r: i64 = r
        .parse()
        .map_err(|_| format!("point « {token} » : ligne illisible"))?;
    let c: i64 = c
        .parse()
        .map_err(|_| format!("point « {token} » : colonne illisible"))?;
    // Un treillis de w×h cases a (w+1)×(h+1) points : les bords comptent.
    if !(0..=h).contains(&r) || !(0..=w).contains(&c) {
        return Err(format!("point « {token} » hors du treillis {w}×{h}"));
    }
    Ok((r, c))
}

/// Découpe un segment en arêtes d'une case. C'est ICI que « un long trait » et
/// « trois courts » deviennent le même dessin.
///
/// Seules trois pentes existent sur ces grilles — horizontale, verticale,
/// diagonale à 45°. Une pente bâtarde (deux cases à droite, une en bas) ne se
/// trace pas sur du papier quadrillé et ne se corrigerait pas : refusée.
fn parse_segment(token: &str, w: i64, h: i64) -> Result<Vec<Edge>, String> {
    let (a, b) = token
        .split_once('-')
        .ok_or_else(|| format!("segment « {token} » illisible (attendu « r,c-r,c »)"))?;
    let a = parse_vertex(a, w, h)?;
    let b = parse_vertex(b, w, h)?;

    let (dr, dc) = (b.0 - a.0, b.1 - a.1);
    if dr == 0 && dc == 0 {
        return Err(format!("segment « {token} » : les deux extrémités sont le même point"));
    }
    if dr != 0 && dc != 0 && dr.abs() != dc.abs() {
        return Err(format!(
            "segment « {token} » : ni horizontal, ni vertical, ni diagonal à 45°"
        ));
    }

    let steps = dr.abs().max(dc.abs());
    let (sr, sc) = (dr / steps, dc / steps);
    let mut out = Vec::with_capacity(steps as usize);
    for i in 0..steps {
        let p = (a.0 + i * sr, a.1 + i * sc);
        let q = (a.0 + (i + 1) * sr, a.1 + (i + 1) * sc);
        out.push(Edge::new(p, q));
    }
    Ok(out)
}

// ===== Rendu SVG ============================================================
//
// Attributs de présentation en dur plutôt que classes CSS, et c'est délibéré :
// ces SVG sont inclus dans la page du contrôle (quiz.css), dans la page de
// correction, ET dans le panel admin (Pico.css). Une figure autonome s'affiche
// pareil partout sans qu'on ait à tenir la même palette dans trois feuilles de
// style — et sans qu'un thème admin puisse rendre un « vert = juste » invisible.

/// Unités SVG par case. Le viewBox suit la grille ; la taille réelle est fixée
/// en CSS par le cadre qui l'accueille.
const U: i64 = 10;
/// Marge autour du quadrillage, en unités. Sans elle, les traits du bord
/// seraient coupés en deux par le viewBox. Elle doit rester supérieure à la
/// demi-épaisseur du trait le plus gros (2.4/2), sinon la figure déborde.
const MARGIN: i64 = 2;

const C_GRID: &str = "#e2dccf";
const C_DOT: &str = "#cfc6b4";
const C_INK: &str = "#243049";
const C_OK: &str = "#2fae66";
const C_EXTRA: &str = "#e07a5f";
const C_MISS: &str = "#aab2c4";

impl Grid {
    /// La figure telle qu'elle est : le modèle à recopier.
    pub fn svg(&self) -> String {
        let mut out = self.svg_open("Modèle à reproduire");
        self.push_lattice(&mut out);
        match &self.figure {
            Figure::Cells(cells) => push_cells(&mut out, cells.iter().copied(), C_INK, false),
            Figure::Edges(edges) => push_edges(&mut out, edges.iter().copied(), C_INK, false),
        }
        out.push_str("</svg>");
        out
    }

    /// Le fond de l'éditeur : le quadrillage seul, sans une marque.
    pub fn svg_blank(&self) -> String {
        let blank = self.blank_like();
        let mut out = blank.svg_open("Grille vierge");
        blank.push_lattice(&mut out);
        out.push_str("</svg>");
        out
    }

    /// La correction, en trois couleurs :
    ///   vert = juste, rouge = en trop, pointillé gris = oublié.
    ///
    /// `self` est le modèle ; `given` le dessin de l'enfant (`None` = rien
    /// dessiné, tout le modèle apparaît alors en oublié).
    pub fn svg_review(&self, given: Option<&Grid>) -> String {
        let blank = self.blank_like();
        let mut out = blank.svg_open("Ton dessin comparé au modèle");
        blank.push_lattice(&mut out);

        match (&self.figure, given.map(|g| &g.figure)) {
            (Figure::Cells(model), Some(Figure::Cells(drawn))) => {
                push_cells(&mut out, model.difference(drawn).copied(), C_MISS, true);
                push_cells(&mut out, drawn.difference(model).copied(), C_EXTRA, false);
                push_cells(&mut out, model.intersection(drawn).copied(), C_OK, false);
            }
            (Figure::Edges(model), Some(Figure::Edges(drawn))) => {
                push_edges(&mut out, model.difference(drawn).copied(), C_MISS, true);
                push_edges(&mut out, drawn.difference(model).copied(), C_EXTRA, false);
                push_edges(&mut out, model.intersection(drawn).copied(), C_OK, false);
            }
            // Rien dessiné (ou natures incompatibles) : tout le modèle est oublié.
            (Figure::Cells(model), _) => {
                push_cells(&mut out, model.iter().copied(), C_MISS, true)
            }
            (Figure::Edges(model), _) => {
                push_edges(&mut out, model.iter().copied(), C_MISS, true)
            }
        }

        out.push_str("</svg>");
        out
    }

    fn svg_open(&self, label: &str) -> String {
        format!(
            r#"<svg class="g-svg" viewBox="{} {} {} {}" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="{}">"#,
            -MARGIN,
            -MARGIN,
            self.w * U + 2 * MARGIN,
            self.h * U + 2 * MARGIN,
            label
        )
    }

    fn push_lattice(&self, out: &mut String) {
        let (right, bottom) = (self.w * U, self.h * U);
        for r in 0..=self.h {
            let y = r * U;
            let _ = write!(
                out,
                r#"<line x1="0" y1="{y}" x2="{right}" y2="{y}" stroke="{C_GRID}" stroke-width=".7"/>"#
            );
        }
        for c in 0..=self.w {
            let x = c * U;
            let _ = write!(
                out,
                r#"<line x1="{x}" y1="0" x2="{x}" y2="{bottom}" stroke="{C_GRID}" stroke-width=".7"/>"#
            );
        }
        // Les points du treillis, seulement pour les segments : ce sont eux
        // qu'on relie, et sans eux l'enfant ne voit pas où accrocher son trait.
        if matches!(self.figure, Figure::Edges(_)) {
            for r in 0..=self.h {
                for c in 0..=self.w {
                    let _ = write!(
                        out,
                        r#"<circle cx="{}" cy="{}" r=".9" fill="{C_DOT}"/>"#,
                        c * U,
                        r * U
                    );
                }
            }
        }
    }
}

fn push_cells(out: &mut String, cells: impl Iterator<Item = Point>, color: &str, dashed: bool) {
    for (r, c) in cells {
        let (x, y) = (c * U, r * U);
        if dashed {
            let _ = write!(
                out,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1" fill="none" stroke="{color}" stroke-width="1" stroke-dasharray="2 2"/>"#,
                x + 1,
                y + 1,
                U - 2,
                U - 2
            );
        } else {
            let _ = write!(
                out,
                r#"<rect x="{x}" y="{y}" width="{U}" height="{U}" rx="1.2" fill="{color}"/>"#
            );
        }
    }
}

fn push_edges(out: &mut String, edges: impl Iterator<Item = Edge>, color: &str, dashed: bool) {
    for e in edges {
        let dash = if dashed { r#" stroke-dasharray="2.4 2.4""# } else { "" };
        let width = if dashed { "1.8" } else { "2.4" };
        let _ = write!(
            out,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{color}" stroke-width="{width}" stroke-linecap="round"{dash}/>"#,
            e.a.1 * U,
            e.a.0 * U,
            e.b.1 * U,
            e.b.0 * U
        );
    }
}

// ===== Aperçu console =======================================================

impl Grid {
    /// Pour `sesame preview` : une figure qu'on peut lire dans un terminal.
    /// Sans ça, l'aperçu d'une question grille afficherait une payload brute —
    /// c'est-à-dire rien de vérifiable à l'œil.
    pub fn ascii(&self) -> String {
        match &self.figure {
            Figure::Cells(cells) => (0..self.h)
                .map(|r| {
                    (0..self.w)
                        .map(|c| if cells.contains(&(r, c)) { "██" } else { "· " })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Figure::Edges(edges) => {
                // Deux caractères par case : les sommets tombent sur les indices
                // pairs, les arêtes sur les impairs.
                let (rows, cols) = ((self.h * 2 + 1) as usize, (self.w * 2 + 1) as usize);
                let mut canvas = vec![vec![' '; cols]; rows];
                for r in 0..=self.h {
                    for c in 0..=self.w {
                        canvas[(r * 2) as usize][(c * 2) as usize] = '·';
                    }
                }
                for e in edges {
                    let (dr, dc) = (e.b.0 - e.a.0, e.b.1 - e.a.1);
                    let (mr, mc) = ((e.a.0 + e.b.0) as usize, (e.a.1 + e.b.1) as usize);
                    let ch = match (dr, dc) {
                        (0, _) => '─',
                        (_, 0) => '│',
                        (1, 1) => '╲',
                        _ => '╱',
                    };
                    // Les deux diagonales d'une même case partagent le centre.
                    canvas[mr][mc] = match (canvas[mr][mc], ch) {
                        ('╲', '╱') | ('╱', '╲') => '╳',
                        _ => ch,
                    };
                }
                canvas
                    .into_iter()
                    .map(|row| row.into_iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

// ===== L'éditeur : des cases à cocher, pas un canvas ========================

/// Une zone cliquable de la grille vierge. Chacune deviendra un
/// `<input type="checkbox">` : le dessin est donc un FORMULAIRE ordinaire, qui
/// part dans le même POST que le reste du contrôle et fonctionne SANS
/// JavaScript — la promesse tenue par `quiz.js` depuis le début. Le glisser
/// pour peindre plusieurs cases d'un geste n'est qu'un confort par-dessus.
///
/// Effet de bord heureux : `quiz.js` mesure « cette question est-elle
/// répondue ? » par « au moins un champ coché ». Des vraies cases à cocher, et
/// la barre de progression, les pastilles et le récapitulatif marchent sans
/// qu'on touche une ligne de ce fichier.
#[derive(Debug, Clone)]
pub struct Toggle {
    /// Le jeton posté — déjà sous forme canonique.
    pub value: String,
    pub class: &'static str,
    /// Position absolue dans le cadre, en pourcentages : le même dessin tient
    /// dans n'importe quelle taille de grille sans une ligne de JS.
    pub style: String,
}

/// Le rapport largeur/hauteur du cadre, à poser en CSS.
///
/// C'est celui du viewBox, MARGE COMPRISE — et pas simplement `w/h`. Sans ça le
/// SVG serait centré avec des bandes vides sur les côtés, tandis que les cases à
/// cocher, elles, se placeraient sur toute la largeur du cadre : les zones
/// sensibles glisseraient à côté du quadrillage dessiné. Un enfant cliquerait
/// sur une case et en verrait s'allumer une autre.
pub fn frame_aspect(w: i64, h: i64) -> String {
    format!("{} / {}", w * U + 2 * MARGIN, h * U + 2 * MARGIN)
}

/// Toutes les zones cliquables d'une grille vierge `w`×`h`.
///
/// Tout est en % du CADRE, marge comprise, pour coïncider au pixel près avec le
/// SVG posé dessous. Les longueurs — y compris celles des arêtes verticales —
/// sont rapportées à la LARGEUR : légitime parce que le cadre porte le rapport
/// rendu par [`frame_aspect`], qui rend les cases carrées. « Une case de haut »
/// et « une case de large » font alors le même nombre de pixels, et une seule
/// règle CSS couvre les quatre orientations — la rotation fait le reste.
pub fn toggles(kind: &str, w: i64, h: i64) -> Vec<Toggle> {
    let span_w = (w * U + 2 * MARGIN) as f64;
    let span_h = (h * U + 2 * MARGIN) as f64;
    // Une case, et le décalage de la marge, en % du cadre.
    let cw = 100.0 * U as f64 / span_w;
    let ch = 100.0 * U as f64 / span_h;
    let off_w = 100.0 * MARGIN as f64 / span_w;
    let off_h = 100.0 * MARGIN as f64 / span_h;
    // La diagonale d'une case vaut √2 fois son côté.
    let diag = cw * SQRT_2;
    let mut out = Vec::new();

    if kind == KIND_CELLS {
        for r in 0..h {
            for c in 0..w {
                out.push(Toggle {
                    value: format!("{r},{c}"),
                    class: "g-cell",
                    style: format!(
                        "left:{:.4}%;top:{:.4}%;width:{:.4}%;height:{:.4}%",
                        off_w + c as f64 * cw,
                        off_h + r as f64 * ch,
                        cw,
                        ch
                    ),
                });
            }
        }
        return out;
    }

    let mut edge = |a: Point, b: Point, top_row: f64, left_col: f64, rot: i64, len: f64| {
        // Les deux diagonales d'une même case se croisent EN SON CENTRE : au
        // milieu, leurs zones sensibles se recouvrent et le clic devient un pile
        // ou face. On les marque pour que le CSS leur donne une bande plus fine
        // — la zone ambiguë rétrécit, et viser vers une extrémité tranche
        // toujours. L'aperçu fantôme au survol montre le trait avant de le poser.
        let diagonal = rot % 90 != 0;
        out.push(Toggle {
            value: Edge::new(a, b).token(),
            class: if diagonal { "g-edge is-diag" } else { "g-edge" },
            style: format!(
                "left:{:.4}%;top:{:.4}%;--len:{:.4}%;--rot:{}deg",
                off_w + left_col * cw,
                off_h + top_row * ch,
                len,
                rot
            ),
        });
    };

    // Les diagonales d'abord : leurs zones sensibles passent SOUS celles des
    // horizontales et des verticales. Aux abords d'un sommet les quatre se
    // chevauchent, et c'est le trait droit — le plus fréquent — qui doit gagner.
    for r in 0..h {
        for c in 0..w {
            edge((r, c), (r + 1, c + 1), r as f64, c as f64, 45, diag);
            edge((r + 1, c), (r, c + 1), (r + 1) as f64, c as f64, -45, diag);
        }
    }
    for r in 0..=h {
        for c in 0..w {
            edge((r, c), (r, c + 1), r as f64, c as f64, 0, cw);
        }
    }
    for r in 0..h {
        for c in 0..=w {
            edge((r, c), (r + 1, c), r as f64, c as f64, 90, cw);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(g: &Grid) -> &BTreeSet<Edge> {
        match &g.figure {
            Figure::Edges(e) => e,
            _ => panic!("figure de segments attendue"),
        }
    }

    // ===== Forme canonique =====
    // Ces tests sont le cœur du module : chacun décrit un dessin JUSTE qu'une
    // comparaison naïve aurait recalé.

    #[test]
    fn cell_order_does_not_matter() {
        let a = Grid::parse("4x4:c=2,1;0,0;1,3").unwrap();
        let b = Grid::parse("4x4:c=1,3;2,1;0,0").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn segment_direction_does_not_matter() {
        // Tracé de droite à gauche : même trait.
        let a = Grid::parse("4x4:e=0,0-0,1").unwrap();
        let b = Grid::parse("4x4:e=0,1-0,0").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn long_segment_equals_its_unit_pieces() {
        // Un enfant trace d'un geste, l'autre en trois : même dessin.
        let long = Grid::parse("4x4:e=0,0-0,3").unwrap();
        let short = Grid::parse("4x4:e=0,0-0,1;0,1-0,2;0,2-0,3").unwrap();
        assert_eq!(long, short);
        assert_eq!(edges(&long).len(), 3);
    }

    #[test]
    fn long_diagonal_is_decomposed_too() {
        let long = Grid::parse("4x4:e=0,0-2,2").unwrap();
        let short = Grid::parse("4x4:e=0,0-1,1;1,1-2,2").unwrap();
        assert_eq!(long, short);
        assert_eq!(edges(&long).len(), 2);
    }

    #[test]
    fn duplicate_marks_collapse() {
        // Double-clic sur la même case : une seule marque.
        let g = Grid::parse("4x4:c=1,1;1,1;1,1").unwrap();
        assert_eq!(g.marks(), 1);
    }

    #[test]
    fn serialize_round_trips() {
        for raw in [
            "8x8:c=0,0;3,4;7,7",
            "6x6:e=0,0-0,3;1,1-2,2",
            "4x4:c=",
            "4x4:e=",
        ] {
            let g = Grid::parse(raw).unwrap();
            assert_eq!(Grid::parse(&g.serialize()).unwrap(), g, "aller-retour de « {raw} »");
        }
    }

    #[test]
    fn parse_is_forgiving_about_spacing_and_case() {
        let a = Grid::parse(" 4X4 : C = 1,1 ; 2,2 ").unwrap();
        let b = Grid::parse("4x4:c=1,1;2,2").unwrap();
        assert_eq!(a, b);
    }

    // ===== Refus =====

    #[test]
    fn rejects_crooked_slopes() {
        // Deux à droite, une en bas : ça ne se trace pas sur du quadrillage.
        assert!(Grid::parse("4x4:e=0,0-1,2").is_err());
    }

    #[test]
    fn rejects_out_of_bounds() {
        assert!(Grid::parse("4x4:c=4,0").is_err(), "ligne 4 n'existe pas sur 4 cases");
        assert!(Grid::parse("4x4:e=0,0-0,5").is_err());
        // Le treillis, lui, a bien un point d'indice 4 : c'est le bord droit.
        assert!(Grid::parse("4x4:e=4,4-4,3").is_ok());
    }

    #[test]
    fn rejects_degenerate_segment() {
        assert!(Grid::parse("4x4:e=1,1-1,1").is_err());
    }

    #[test]
    fn rejects_absurd_sizes() {
        assert!(Grid::parse("1x1:c=0,0").is_err());
        assert!(Grid::parse("40x40:c=0,0").is_err());
    }

    #[test]
    fn rejects_kind_mismatch() {
        assert!(Grid::parse_as(KIND_LINES, "4x4:c=1,1").is_err());
        assert!(Grid::parse_as(KIND_CELLS, "4x4:c=1,1").is_ok());
    }

    #[test]
    fn rejects_empty_model() {
        // On réussirait la question sans rien dessiner.
        assert!(Grid::validate_model(KIND_CELLS, "4x4:c=").is_err());
        assert!(Grid::validate_model(KIND_CELLS, "4x4:c=1,1").is_ok());
    }

    // ===== Ce que l'enfant envoie =====

    #[test]
    fn tokens_build_the_same_figure_as_a_payload() {
        let model = Grid::parse("4x4:c=1,1;2,2").unwrap();
        let drawn = Grid::from_tokens(
            KIND_CELLS,
            4,
            4,
            &["2,2".into(), "1,1".into()], // ordre des clics : indifférent
        );
        assert_eq!(model, drawn);
    }

    #[test]
    fn tokens_ignore_garbage_without_losing_the_rest() {
        let drawn = Grid::from_tokens(
            KIND_CELLS,
            4,
            4,
            &["1,1".into(), "n'importe quoi".into(), "9,9".into(), "2,2".into()],
        );
        assert_eq!(drawn, Grid::parse("4x4:c=1,1;2,2").unwrap());
    }

    #[test]
    fn empty_submission_never_matches_a_model() {
        let model = Grid::parse("4x4:e=0,0-0,1").unwrap();
        let drawn = Grid::from_tokens(KIND_LINES, 4, 4, &[]);
        assert_ne!(model, drawn);
        assert!(drawn.is_empty());
    }

    #[test]
    fn one_mark_too_many_is_wrong() {
        // 100 % ou rien : c'est la règle de l'exercice.
        let model = Grid::parse("4x4:c=1,1;2,2").unwrap();
        let drawn = Grid::from_tokens(KIND_CELLS, 4, 4, &["1,1".into(), "2,2".into(), "3,3".into()]);
        assert_ne!(model, drawn);
    }

    // ===== Éditeur =====

    #[test]
    fn toggles_cover_every_cell() {
        assert_eq!(toggles(KIND_CELLS, 8, 8).len(), 64);
        assert_eq!(toggles(KIND_CELLS, 4, 6).len(), 24);
    }

    #[test]
    fn toggles_cover_every_unit_edge_exactly_once() {
        for (w, h) in [(4, 4), (6, 6), (8, 8), (4, 6)] {
            let t = toggles(KIND_LINES, w, h);
            // horizontales (h+1)·w + verticales h·(w+1) + 2 diagonales par case
            let expected = ((h + 1) * w + h * (w + 1) + 2 * w * h) as usize;
            assert_eq!(t.len(), expected, "grille {w}×{h}");

            let unique: BTreeSet<&String> = t.iter().map(|x| &x.value).collect();
            assert_eq!(unique.len(), t.len(), "doublon de zone cliquable sur {w}×{h}");
        }
    }

    #[test]
    fn every_toggle_value_is_a_valid_canonical_token() {
        // Si une zone produisait un jeton que le correcteur rejette, l'enfant
        // dessinerait une marque qui ne compte pas — invisible et imparable.
        for kind in [KIND_CELLS, KIND_LINES] {
            for t in toggles(kind, 8, 8) {
                let g = Grid::from_tokens(kind, 8, 8, &[t.value.clone()]);
                assert_eq!(g.marks(), 1, "jeton « {} » perdu en route", t.value);
            }
        }
    }
}
