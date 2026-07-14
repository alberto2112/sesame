/* sesame — le contrôle en cartes : une question à la fois.
 *
 * Tout se passe dans le navigateur : les N questions sont déjà dans le DOM et
 * dans le <form>. On n'en montre qu'une, mais le formulaire garde les réponses
 * tout seul — revenir en arrière ne coûte rien et l'envoi reste UN seul POST
 * /submit. Le serveur ne connaît pas la notion d'« examen à moitié fait ».
 *
 * Sans JS, la page reste utilisable : tout est visible, un seul bouton.
 */
(() => {
  const form = document.getElementById("quiz");
  if (!form) return;

  /* --- « cette carte est-elle répondue ? » --------------------------------
   * Par défaut : au moins un champ coché ou rempli. Cette règle couvre les
   * types actuels ('single', 'multi' — voir le CHECK de la table questions)
   * et tout type futur bâti sur des champs de formulaire.
   *
   * Point d'extension : un type qui ne se mesure pas ainsi (ordre à remettre,
   * appariement…) inscrit sa règle dans ANSWERED, indexée par son `data-kind`.
   * Le défaut n'est JAMAIS « répondu » : un type mal orthographié doit sauter
   * aux yeux, pas ouvrir une porte dérobée qui laisse passer une carte vide. */
  const ANSWERED = {};

  const isFilled = (f) =>
    f.type === "radio" || f.type === "checkbox" ? f.checked : f.value.trim() !== "";

  const isAnswered = (card) => {
    const rule = ANSWERED[card.dataset.kind];
    if (rule) return rule(card);
    return [...card.querySelectorAll("input, select, textarea")].some(isFilled);
  };

  const cards = [...form.querySelectorAll("[data-card]")];
  const questions = cards.filter((c) => !c.hasAttribute("data-recap"));
  const total = questions.length;
  if (total === 0) return;

  const recapIndex = total; // la dernière carte est le récapitulatif
  const stepLabel = form.querySelector("[data-step-label]");
  const answeredLabel = form.querySelector("[data-answered-label]");
  const progress = form.querySelector("[data-progress]");
  const dotsBox = form.querySelector("[data-dots]");
  const warn = form.querySelector("[data-warn]");
  const btnPrev = form.querySelector("[data-prev]");
  const btnNext = form.querySelector("[data-next]");
  const btnFinal = form.querySelector("[data-final]");
  const recapStates = [...form.querySelectorAll("[data-recap-state]")];

  // Une pastille par question : état d'un coup d'œil + raccourci pour y revenir.
  const dots = questions.map((_, i) => {
    const dot = document.createElement("button");
    dot.type = "button";
    dot.className = "dot";
    dot.textContent = String(i + 1);
    dot.setAttribute("aria-label", `Question ${i + 1}`);
    dot.addEventListener("click", () => go(i));
    dotsBox.appendChild(dot);
    return dot;
  });

  let current = 0;

  function go(i) {
    current = Math.max(0, Math.min(recapIndex, i));
    cards.forEach((c, k) => c.classList.toggle("is-active", k === current));
    warn.hidden = true;
    refresh();
    form.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function refresh() {
    const done = questions.filter(isAnswered).length;
    const onRecap = current === recapIndex;

    stepLabel.textContent = onRecap
      ? "Récapitulatif"
      : `Question ${current + 1} / ${total}`;
    answeredLabel.textContent = `${done} / ${total} répondues`;
    progress.style.width = `${((onRecap ? total : current) / total) * 100}%`;

    dots.forEach((dot, i) => {
      dot.classList.toggle("is-current", i === current);
      dot.classList.toggle("is-done", isAnswered(questions[i]));
    });

    recapStates.forEach((el, i) => {
      const ok = isAnswered(questions[i]);
      el.textContent = ok ? "✓" : "?";
      el.closest(".recap-item").classList.toggle("is-missing", !ok);
    });

    btnPrev.disabled = current === 0;
    btnNext.hidden = onRecap;
    btnFinal.hidden = !onRecap;
  }

  // Cocher une réponse met à jour les pastilles, sans faire avancer tout seul :
  // l'enfant garde la main, il peut changer d'avis avant de passer à la suite.
  form.addEventListener("change", refresh);

  btnPrev.addEventListener("click", () => go(current - 1));
  btnNext.addEventListener("click", () => go(current + 1));
  form.querySelectorAll("[data-goto]").forEach((b) =>
    b.addEventListener("click", () => go(Number(b.dataset.goto)))
  );

  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, button")) return;
    if (e.key === "ArrowRight") go(current + 1);
    if (e.key === "ArrowLeft") go(current - 1);
  });

  // Validation finale : on ne poste pas un contrôle troué par distraction.
  // (Les `required` HTML sont impossibles ici : un champ caché en display:none
  // n'est pas focusable et le navigateur refuse alors de soumettre, en silence.)
  form.addEventListener("submit", (e) => {
    const missing = questions.findIndex((c) => !isAnswered(c));
    if (missing === -1) return;
    e.preventDefault();
    go(missing);
    warn.textContent = `Il te manque la question ${missing + 1}. Réponds-y avant de valider !`;
    warn.hidden = false;
  });

  go(0);
})();
