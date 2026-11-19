"use strict";
// Les réglages (refonte) : un menu en trois groupes — cet appareil, la maison,
// l'administration — et une rubrique à la fois. Sur un téléphone, le menu
// d'abord, puis la rubrique avec un retour.
//
// account.js remplit et montre les blocs comme avant (mêmes id) ; ici, une
// rubrique n'est cachée que si tout son contenu l'est, et un groupe que si
// toutes ses rubriques le sont. Le style est en classes Tailwind (css/app.css) :
// data-view="list" ou "detail" sur le dialogue choisit ce que montre un
// téléphone. Pas d'innerHTML.
(() => {
  const dialog = document.getElementById("account-dialog");
  if (!dialog) return;
  const links = [...dialog.querySelectorAll(".set-link")];
  const panes = [...dialog.querySelectorAll(".set-pane")];
  const narrow = matchMedia("(max-width: 720px)");
  const paneOf = (name) => panes.find((p) => p.dataset.pane === name);

  // Les cases à cocher en interrupteurs, le texte d'abord. Écrit en entier
  // ici pour que Tailwind trouve les classes ; posé aussi sur les cases
  // ajoutées plus tard (les scènes du hub).
  const SWITCH = [
    "relative", "order-2", "ml-auto", "shrink-0", "w-[2.6rem]", "h-6", "m-0", "rounded-full",
    "bg-line", "appearance-none", "cursor-pointer", "transition-colors", "checked:bg-ok",
    "after:absolute", "after:top-[3px]", "after:left-[3px]", "after:size-[18px]",
    "after:rounded-full", "after:bg-white", "after:shadow", "after:transition-transform",
    "checked:after:translate-x-[1.1rem]", "motion-reduce:transition-none",
    "motion-reduce:after:transition-none",
  ];
  const switches = () => {
    for (const input of dialog.querySelectorAll('label.check input[type="checkbox"]')) input.classList.add(...SWITCH);
  };

  // Un bloc compte s'il n'est pas caché (ni le bouton qu'il accompagne).
  const shown = (block) => {
    if (block.hidden || block.tagName === "HEADER") return false;
    const follow = block.dataset.follow && document.getElementById(block.dataset.follow);
    return !follow || !follow.hidden;
  };

  function refresh() {
    for (const link of links) link.hidden = ![...paneOf(link.dataset.pane).children].some(shown);
    for (const group of dialog.querySelectorAll(".set-group")) {
      let any = false;
      for (let n = group.nextElementSibling; n && n.classList.contains("set-link"); n = n.nextElementSibling) {
        if (!n.hidden) any = true;
      }
      group.hidden = !any;
    }
  }

  function show(name, detail) {
    for (const pane of panes) pane.hidden = pane.dataset.pane !== name;
    for (const link of links) {
      if (link.dataset.pane === name) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    }
    dialog.dataset.view = detail ? "detail" : "list";
  }

  function current() {
    const link = links.find((l) => l.getAttribute("aria-current") === "page" && !l.hidden);
    return link ? link.dataset.pane : (links.find((l) => !l.hidden) || links[0]).dataset.pane;
  }

  // À l'ouverture : la liste sur un téléphone, la rubrique en cours ailleurs.
  function opened() {
    switches();
    refresh();
    show(current(), !narrow.matches);
  }

  for (const link of links) {
    link.addEventListener("click", () => {
      show(link.dataset.pane, true);
      const pane = paneOf(link.dataset.pane);
      if (narrow.matches) pane.querySelector("h3").focus({ preventScroll: true });
    });
  }
  dialog.querySelector(".set-back").addEventListener("click", () => {
    dialog.dataset.view = "list";
    const link = links.find((l) => l.getAttribute("aria-current") === "page");
    if (link) link.focus();
  });
  narrow.addEventListener("change", () => { if (dialog.open) show(current(), true); });

  // account.js montre les blocs au fil des réponses du serveur.
  new MutationObserver(refresh).observe(dialog.querySelector(".set-panes"), {
    attributes: true, attributeFilter: ["hidden"], subtree: true,
  });
  new MutationObserver(() => { if (dialog.open) opened(); }).observe(dialog, {
    attributes: true, attributeFilter: ["open"],
  });

  // Les vignettes du thème pilotent la liste cachée, que lit déjà account.js.
  const select = document.getElementById("theme");
  const tiles = [...dialog.querySelectorAll(".theme-tile")];
  const syncTiles = () => {
    for (const tile of tiles) tile.setAttribute("aria-checked", String(tile.dataset.theme === select.value));
  };
  for (const tile of tiles) {
    tile.addEventListener("click", () => {
      select.value = tile.dataset.theme;
      select.dispatchEvent(new Event("change", { bubbles: true }));
      syncTiles();
    });
  }
  select.addEventListener("change", syncTiles);
  syncTiles();
  switches();
})();
