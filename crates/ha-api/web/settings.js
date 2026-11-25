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
    touch();
  };

  // Des cibles de 44 px au moins, au doigt : les rubriques, le retour, les
  // interrupteurs (toute la ligne), les petits boutons, les volets dépliants.
  // Posées ici, comme les interrupteurs, sur les blocs de index.html et sur
  // ceux qu'account.js ajoute plus tard.
  const touch = () => {
    for (const node of dialog.querySelectorAll("label.check, button.small, #logout, .set-back")) node.classList.add("min-h-11");
    for (const node of dialog.querySelectorAll("summary")) node.classList.add("py-3");
  };
  for (const link of links) {
    link.classList.add("min-h-11", "flex", "items-center", "justify-between", "gap-2");
    // Sur un téléphone, la rubrique s'ouvre en entier : un chevron le dit.
    link.append(el("span", { class: "hidden max-[720px]:inline-grid place-items-center text-muted [[aria-current=page]>&]:text-accent-ink [&_svg]:w-4 [&_svg]:h-4", "aria-hidden": "true" }, svgIcon(["M9 6l6 6-6 6"])));
  }
  dialog.querySelector(".set-back").classList.add("rounded-champ", "px-4");

  // La rubrique ouverte est retenue d'une fois sur l'autre (sur cet appareil).
  const PANE_KEY = "ha-reglages-rubrique";
  const remembered = () => {
    try { return localStorage.getItem(PANE_KEY); } catch { return null; }
  };
  const remember = (name) => {
    try { localStorage.setItem(PANE_KEY, name); } catch { /* navigation privée */ }
  };

  // « Enregistré », à côté du réglage : ces réglages s'appliquent tout de
  // suite (sur cet appareil), il n'y a pas de bouton Enregistrer à chercher.
  // Le texte visible est doublé d'une annonce discrète pour les lecteurs d'écran.
  const announce = el("p", { class: "sr-only", role: "status" });
  dialog.append(announce);
  function saved(anchor, text = "Enregistré") {
    dialog.querySelector(".set-enregistre")?.remove();
    clearTimeout(saved.timer);
    const pill = el("span", { class: "set-enregistre inline-flex items-center gap-1 py-0.5 px-2 rounded-full bg-ok-soft text-text text-legende font-semibold whitespace-nowrap [&_svg]:w-3.5 [&_svg]:h-3.5", "aria-hidden": "true" }, svgIcon(["M5 12l5 5L20 7"]), text);
    if (anchor.tagName === "LABEL" && anchor.classList.contains("check")) {
      pill.classList.add("order-1", "ml-auto");
      anchor.append(pill);
    } else {
      pill.classList.add("justify-self-start");
      anchor.after(pill);
    }
    announce.textContent = text;
    saved.timer = setTimeout(() => {
      pill.remove();
      announce.textContent = "";
    }, 2500);
  }
  dialog.querySelector(".set-panes").addEventListener("change", (event) => {
    const input = event.target;
    // Les notifications passent par le serveur : account.js dit lui-même quand c'est fait.
    if (!(input instanceof HTMLElement) || input.closest("#push")) return;
    if (input.id === "theme") return saved(dialog.querySelector('[role="radiogroup"]'));
    if (input.matches('label.check input[type="checkbox"]')) return saved(input.closest("label"));
    if (input.tagName === "SELECT") return saved(input.closest("label") ?? input);
    if (input.closest("#hub-pick-list")) return saved(dialog.querySelector("#hub-pick summary"), "Scènes enregistrées");
  });

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
    remember(name);
  }

  function current() {
    const link = links.find((l) => l.getAttribute("aria-current") === "page" && !l.hidden)
      ?? links.find((l) => l.dataset.pane === remembered() && !l.hidden);
    return link ? link.dataset.pane : (links.find((l) => !l.hidden) || links[0]).dataset.pane;
  }

  // À l'ouverture : la liste sur un téléphone (la rubrique retenue y est
  // marquée), la rubrique retenue ailleurs.
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
