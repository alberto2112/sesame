/* sesame — l'éditeur de grille du panel d'administration.
 *
 * Le parent DESSINE le modèle au lieu de taper « 8x8:e=0,0-0,1;… » à la main.
 * Ce script ne fait que trois choses :
 *   1. montrer l'éditeur quand le type choisi est une grille ;
 *   2. afficher la planche qui correspond au type ET à la taille demandés ;
 *   3. recopier les cases cochées dans le champ « Réponse 1 ».
 *
 * Ce qu'il ne fait PAS, et c'est le point important : il ne calcule aucune
 * géométrie et ne décide de rien. Où tombe une arête, quel jeton elle porte,
 * quelles tailles existent — tout cela est écrit en Rust (`grid::toggles`,
 * `admin.rs::build_grid_boards`) et rendu par le serveur. Le texte qu'il produit
 * n'est qu'un transport : le serveur le reparse et le remet sous forme
 * canonique avant de l'enregistrer. Une seule autorité sur ce qu'est un dessin.
 *
 * Sans JavaScript, le bloc reste caché et le champ texte accepte toujours la
 * figure écrite à la main : l'éditeur est un confort, pas une dépendance.
 */
(() => {
  const editor = document.querySelector("[data-grid-editor]");
  if (!editor) return;

  const kindSelect = document.querySelector("select[name=kind]");
  const sizeSelect = editor.querySelector("[data-grid-size]");
  const target = document.querySelector("input[name=ans_1_text]");
  const correct = document.querySelector("input[name=ans_1_correct]");
  const boards = [...editor.querySelectorAll("[data-grid-board]")];
  if (!kindSelect || !sizeSelect || !target || !boards.length) return;

  const isGrid = () => kindSelect.value.startsWith("grid_");
  const activeBoard = () =>
    boards.find((b) => b.dataset.gridBoard === `${kindSelect.value}:${sizeSelect.value}`);

  /* Le champ texte est la SOURCE : c'est lui qui part au serveur. On l'écrit à
     chaque coup de crayon, et on coche « correcte » au passage — une figure non
     cochée serait refusée à l'enregistrement, et le parent chercherait pourquoi. */
  const publish = () => {
    const board = activeBoard();
    if (!board) return;
    const marks = [...board.querySelectorAll("input:checked")].map((b) => b.value);
    target.value = `${sizeSelect.value}:${kindSelect.value === "grid_cells" ? "c" : "e"}=${marks.join(";")}`;
    if (correct) correct.checked = true;
  };

  /* Relire une figure déjà enregistrée, pour que « Modifier » ouvre le dessin
     et non une grille vide. Format : « 8x8:c=0,3;1,3 ». On règle la taille et le
     type d'après la figure elle-même — elle est autodescriptive, c'est fait
     pour. Un jeton qui ne correspond à aucune case est ignoré sans bruit : le
     serveur reste seul juge de ce qui est valide. */
  const load = () => {
    const raw = (target.value || "").replace(/\s+/g, "");
    const m = raw.match(/^(\d+x\d+):([ce])=(.*)$/i);
    if (!m) return;
    const [, size, marker, body] = m;
    const kind = marker.toLowerCase() === "c" ? "grid_cells" : "grid_lines";
    if (kindSelect.value !== kind) return; // le type déclaré fait foi
    if ([...sizeSelect.options].some((o) => o.value === size)) sizeSelect.value = size;

    const board = activeBoard();
    if (!board) return;
    const wanted = new Set(body.split(";").filter(Boolean));
    board.querySelectorAll("input").forEach((b) => {
      b.checked = wanted.has(b.value);
    });
  };

  const show = () => {
    editor.hidden = !isGrid();
    const active = activeBoard();
    boards.forEach((b) => {
      b.hidden = b !== active;
    });
  };

  boards.forEach((board) => {
    const grid = board.querySelector("[data-grid]");
    if (grid && window.sesameGridPaint) window.sesameGridPaint(grid, publish);
    // Le clavier (Espace sur une case) ne passe pas par sesameGridPaint : il
    // bascule la case nativement, et il faut quand même republier.
    if (grid) grid.addEventListener("change", publish);
  });

  kindSelect.addEventListener("change", () => {
    show();
    if (isGrid()) publish();
  });
  sizeSelect.addEventListener("change", () => {
    show();
    publish();
  });

  // `load` peut changer la taille : on n'affiche qu'après, sinon la planche
  // visible ne serait pas celle qu'on vient de remplir.
  load();
  show();
})();
