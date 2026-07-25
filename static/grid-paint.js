/* sesame — peindre une grille au glisser.
 *
 * Le même geste sert à deux endroits : l'enfant qui reproduit un dessin pendant
 * le contrôle (quiz.js), et le parent qui compose le modèle dans le panel
 * d'administration (grid-editor.js). D'où ce fichier plutôt qu'un bloc recopié
 * dans chacun : deux copies auraient dérivé, et un dessin ne se serait plus
 * tracé pareil des deux côtés du même exercice.
 *
 * ⚠️  RIEN ICI N'EST NÉCESSAIRE POUR DESSINER. La grille est faite de vraies
 * cases à cocher dans un vrai <label> : sans une ligne de JavaScript, on clique
 * et ça marche. Ce fichier n'ajoute que le confort de balayer au lieu de
 * cliquer trente fois — c'est un supplément, jamais une dépendance.
 *
 * `onChange` est appelé après CHAQUE bascule : la page du contrôle s'en sert
 * pour rafraîchir sa barre de progression, qui ne verrait rien passer sinon.
 */
(() => {
  window.sesameGridPaint = function (grid, onChange) {
    const notify = typeof onChange === "function" ? onChange : () => {};
    // true = on coche sur son passage, false = on décoche, null = repos.
    let painting = null;

    const boxAt = (el) => {
      const label = el && el.closest ? el.closest("label") : null;
      if (!label || label.parentElement !== grid) return null;
      return label.querySelector("input");
    };

    const paint = (box) => {
      if (!box || box.checked === painting) return;
      box.checked = painting;
      notify();
    };

    grid.addEventListener("pointerdown", (e) => {
      const box = boxAt(e.target);
      if (!box) return;
      e.preventDefault(); // pas de sélection de texte pendant qu'on dessine
      painting = !box.checked;
      paint(box);
      if (grid.setPointerCapture) grid.setPointerCapture(e.pointerId);
    });

    grid.addEventListener("pointermove", (e) => {
      if (painting === null) return;
      // La capture du pointeur ramène TOUS les évènements sur le cadre : à nous
      // de retrouver la case réellement sous le curseur.
      paint(boxAt(document.elementFromPoint(e.clientX, e.clientY)));
    });

    // Puisque nous basculons la case nous-mêmes au pointerdown, il FAUT annuler
    // l'activation native du <label> au clic : sinon elle rebasculerait aussitôt
    // et le clic n'aurait aucun effet visible.
    grid.addEventListener("click", (e) => {
      // `detail === 0` = clic fabriqué par le clavier (Espace sur la case) :
      // celui-là, on le laisse vivre — c'est le seul geste possible sans souris.
      if (e.detail !== 0 && boxAt(e.target)) e.preventDefault();
    });

    const rest = () => {
      painting = null;
    };
    document.addEventListener("pointerup", rest);
    document.addEventListener("pointercancel", rest);

    // Le bouton « Tout effacer » vit à côté du cadre, pas dedans.
    const parent = grid.parentElement;
    const clear = parent && parent.querySelector("[data-grid-clear]");
    if (clear) {
      clear.addEventListener("click", () => {
        grid.querySelectorAll("input:checked").forEach((b) => (b.checked = false));
        notify();
      });
    }
  };
})();
