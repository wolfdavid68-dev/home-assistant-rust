"use strict";
// ---------------------------------------------------------------- Schéma
// L'éditeur d'automatisations en blocs reliés, façon n8n (étape 35).
//
// Le schéma n'est qu'une autre **vue** de la même configuration : à chaque
// modification, il est retransformé en configuration (`graphToConfig`), que
// le serveur vérifie comme celle du formulaire ou du YAML. Seule la place des
// blocs est rangée à part (PUT /api/automations/{id}/layout).
//
// Une automatisation est un arbre (déclencheurs → conditions → actions, avec
// des branches « si / sinon », « selon le cas », « répéter ») : chaque bloc a
// donc **une seule** entrée, et un fil par sortie. Les déclencheurs, eux,
// arrivent tous sur le même bloc (le début).
//
// Ce fichier est chargé après app.js et en réutilise les fonctions (el, svg,
// itemsBlock, triggerFields, describeAction…) : les deux partagent la même
// portée globale, un nom ne doit donc pas être déclaré deux fois.

const NODE_W = 236;
const NODE_MIN_H = 76;
const PORT_GAP = 30;
const COL = 300;
const ROW = 140;
const MIN_SCALE = 0.3;
const MAX_SCALE = 2;

const canvas = {
  nodes: new Map(), // clé → bloc
  wires: [], // { from, port, to }
  nextKey: 1,
  view: { x: 40, y: 40, scale: 1 },
  selected: null, // clé du bloc ouvert dans le panneau
  selectedWire: null,
  // Le dessin des blocs a changé depuis le chargement : les noms de place
  // (« 2 › alors › 1 ») ne correspondent plus à ceux enregistrés.
  structureChanged: false,
  runs: [], // exécutions (historique)
  run: null, // exécution affichée sur le schéma
  pointers: new Map(), // doigts posés (pincement)
  drag: null,
  layoutTimer: null,
  readOnly: false,
};

// Les sorties de chaque sorte de bloc : [nom, libellé].
function outputs(node) {
  if (node.kind === "trigger") return [["next", ""]];
  if (node.kind === "condition") return [["next", "si vrai"]];
  switch (actionKind(node.item)) {
    case "if":
      return [["then", "alors"], ["else", "sinon"], ["next", "ensuite"]];
    case "choose":
      return [
        ...(node.item.choose ?? []).map((_, i) => [`opt${i}`, `option ${i + 1}`]),
        ["default", "par défaut"],
        ["next", "ensuite"],
      ];
    case "repeat":
      return [["body", "à répéter"], ["next", "ensuite"]];
    case "condition":
      return [["next", "si vrai"]];
    default:
      return [["next", ""]];
  }
}

const hasInput = (node) => node.kind !== "trigger";
const nodeHeight = (node) => Math.max(NODE_MIN_H, 34 + outputs(node).length * PORT_GAP);

function nodeTitle(node) {
  if (node.kind === "trigger") return ["Quand", lookup(TRIGGER_KINDS, node.item.trigger)?.[0] ?? node.item.trigger];
  if (node.kind === "condition") return ["Si", lookup(CONDITION_KINDS, node.item.condition)?.[0] ?? node.item.condition];
  const kind = actionKind(node.item);
  return ["Alors", lookup(ACTION_KINDS, kind)?.[0] ?? kind];
}

function nodeText(node) {
  try {
    if (node.kind === "trigger") return describeTrigger(node.item);
    if (node.kind === "condition") return describeCondition(node.item);
    switch (actionKind(node.item)) {
      case "if":
        return node.item.if?.length ? `si ${node.item.if.map(describeCondition).join(" et ")}` : "conditions à choisir";
      case "choose":
        return `${(node.item.choose ?? []).length} option(s)`;
      case "repeat": {
        const r = node.item.repeat ?? {};
        if (r.count !== undefined) return `${r.count} fois`;
        if (r.while) return `tant que ${r.while.map(describeCondition).join(" et ")}`;
        if (r.until) return `jusqu'à ce que ${r.until.map(describeCondition).join(" et ")}`;
        return "";
      }
      default:
        return describeAction(node.item);
    }
  } catch {
    return "";
  }
}

function nodeIcon(node) {
  if (node.kind === "trigger") return ICONS.play;
  if (node.kind === "condition") return ICONS.shield;
  switch (actionKind(node.item)) {
    case "delay":
    case "wait_for_trigger":
      return ICONS.timer;
    case "if":
    case "choose":
    case "condition":
      return ICONS.shield;
    case "repeat":
      return ICONS.sparkle;
    default:
      return ICONS.bulb;
  }
}

// ----- Configuration → schéma

/** Les actions sans leurs suites (`then`, `sequence`…), portées par les fils. */
function stripBranches(item) {
  const copy = structuredClone(item);
  switch (actionKind(copy)) {
    case "if":
      delete copy.then;
      delete copy.else;
      break;
    case "choose":
      copy.choose = (copy.choose ?? []).map((option) => ({ conditions: option.conditions ?? [] }));
      delete copy.default;
      break;
    case "repeat":
      if (copy.repeat) delete copy.repeat.sequence;
      break;
  }
  return copy;
}

function addNode(kind, item, x, y, path = null) {
  const key = `n${canvas.nextKey++}`;
  canvas.nodes.set(key, { key, kind, item, x, y, path });
  return key;
}

function connect(from, port, to) {
  // Une seule entrée par bloc (sauf les déclencheurs, tous vers le début),
  // un seul fil par sortie.
  canvas.wires = canvas.wires.filter((w) => !(w.from === from && w.port === port));
  const target = canvas.nodes.get(to);
  const source = canvas.nodes.get(from);
  // L'entrée d'un bloc ne reçoit qu'un fil d'un autre bloc (les fils des
  // déclencheurs, eux, s'ajoutent).
  if (source?.kind !== "trigger") {
    canvas.wires = canvas.wires.filter((w) => w.to !== to || canvas.nodes.get(w.from)?.kind === "trigger");
  }
  if (target && hasInput(target)) canvas.wires.push({ from, port, to });
}

const wireFrom = (key, port) => canvas.wires.find((w) => w.from === key && w.port === port)?.to ?? null;

/** Construit le schéma d'une configuration, avec la place enregistrée des blocs. */
function configToGraph(config, layout = {}) {
  canvas.nodes = new Map();
  canvas.wires = [];
  canvas.nextKey = 1;
  canvas.structureChanged = false;
  const place = (path, x, y) => {
    const saved = Array.isArray(layout?.[path]) ? layout[path] : null;
    return saved && saved.every(Number.isFinite) ? saved : [x, y];
  };

  // Les déclencheurs, en colonne.
  const triggers = (config.triggers ?? []).map((t, i) => {
    const path = `déclencheur ${i + 1}`;
    const [x, y] = place(path, 0, i * ROW);
    return addNode("trigger", structuredClone(t), x, y, path);
  });

  // Une suite d'actions, de gauche à droite ; les branches en dessous.
  // Renvoie [clé du premier bloc, hauteur en lignes, largeur en colonnes].
  const chain = (items, prefix, col, row) => {
    let first = null;
    let previous = null;
    let height = 1;
    let x = col;
    items.forEach((item, index) => {
      const path = prefix === "" ? String(index + 1) : `${prefix} › ${index + 1}`;
      const [px, py] = place(path, x * COL, row * ROW);
      const key = addNode("action", stripBranches(item), px, py, path);
      if (previous) connect(previous, "next", key);
      else first = key;
      previous = key;
      // Les branches : chacune sur ses lignes, à droite du bloc.
      const branches = [];
      switch (actionKind(item)) {
        case "if":
          branches.push(["then", item.then ?? [], `${path} › alors`]);
          branches.push(["else", item.else ?? [], `${path} › sinon`]);
          break;
        case "choose":
          (item.choose ?? []).forEach((option, i) => branches.push([`opt${i}`, option.sequence ?? [], `${path} › option ${i + 1}`]));
          branches.push(["default", item.default ?? [], `${path} › par défaut`]);
          break;
        case "repeat":
          branches.push(["body", item.repeat?.sequence ?? [], `${path} › boucle`]);
          break;
      }
      let branchRow = row;
      let width = 1;
      for (const [port, list, branchPath] of branches) {
        if (list.length === 0) continue;
        const [branchFirst, h, w] = chain(list, branchPath, x + 1, branchRow);
        connect(key, port, branchFirst);
        branchRow += h;
        width = Math.max(width, 1 + w);
      }
      height = Math.max(height, branchRow - row, 1);
      x += width;
    });
    return [first, height, x - col];
  };

  // Les conditions de l'automatisation, puis ses actions.
  let col = triggers.length > 0 ? 1 : 0;
  let previous = null;
  let start = null;
  (config.conditions ?? []).forEach((c, i) => {
    const path = `condition ${i + 1}`;
    const [x, y] = place(path, col * COL, 0);
    const key = addNode("condition", structuredClone(c), x, y, path);
    if (previous) connect(previous, "next", key);
    else start = key;
    previous = key;
    col += 1;
  });
  const [firstAction] = chain(config.actions ?? [], "", col, 0);
  if (firstAction) {
    if (previous) connect(previous, "next", firstAction);
    else start = firstAction;
  }
  if (start) for (const t of triggers) connect(t, "next", start);
}

// ----- Schéma → configuration

/**
 * La configuration dessinée, et la place de chaque bloc sous son nom (« 2 ›
 * alors › 1 »). `errors` : ce qui empêche de l'enregistrer.
 */
function graphToConfig(base) {
  const errors = [];
  const layout = {};
  const visited = new Set();
  const config = { ...base };
  delete config.triggers;
  delete config.conditions;
  delete config.actions;

  const nodes = [...canvas.nodes.values()];
  const triggers = nodes.filter((n) => n.kind === "trigger");
  config.triggers = triggers.map((t, i) => {
    t.path = `déclencheur ${i + 1}`;
    layout[t.path] = [t.x, t.y];
    visited.add(t.key);
    return structuredClone(t.item);
  });
  if (triggers.length === 0) errors.push("Ajoute un déclencheur (« Quand… »).");
  const starts = new Set(triggers.map((t) => wireFrom(t.key, "next")));
  if (starts.has(null)) errors.push("Chaque déclencheur doit être relié au premier bloc.");
  starts.delete(null);
  if (starts.size > 1) errors.push("Tous les déclencheurs doivent arriver sur le même bloc.");
  let key = [...starts][0] ?? null;

  // Les conditions du début de la chaîne principale.
  config.conditions = [];
  while (key && canvas.nodes.get(key)?.kind === "condition") {
    if (visited.has(key)) {
      errors.push("Le schéma tourne en rond.");
      key = null;
      break;
    }
    const node = canvas.nodes.get(key);
    visited.add(key);
    node.path = `condition ${config.conditions.length + 1}`;
    layout[node.path] = [node.x, node.y];
    config.conditions.push(structuredClone(node.item));
    key = wireFrom(key, "next");
  }
  if (config.conditions.length === 0) delete config.conditions;

  // Une suite de blocs, à partir de `first`.
  const follow = (first, prefix) => {
    const list = [];
    let current = first;
    while (current) {
      if (visited.has(current)) {
        errors.push("Le schéma tourne en rond.");
        break;
      }
      visited.add(current);
      const node = canvas.nodes.get(current);
      const path = prefix === "" ? String(list.length + 1) : `${prefix} › ${list.length + 1}`;
      node.path = path;
      layout[path] = [node.x, node.y];
      // Une condition ajoutée plus loin dans la chaîne : « continuer seulement si… ».
      const item = structuredClone(node.item);
      switch (node.kind === "action" ? actionKind(item) : "condition") {
        case "if": {
          item.then = follow(wireFrom(current, "then"), `${path} › alors`);
          const otherwise = follow(wireFrom(current, "else"), `${path} › sinon`);
          if (otherwise.length > 0) item.else = otherwise;
          break;
        }
        case "choose":
          item.choose = (item.choose ?? []).map((option, i) => ({
            conditions: option.conditions ?? [],
            sequence: follow(wireFrom(current, `opt${i}`), `${path} › option ${i + 1}`),
          }));
          {
            const fallback = follow(wireFrom(current, "default"), `${path} › par défaut`);
            if (fallback.length > 0) item.default = fallback;
          }
          break;
        case "repeat":
          item.repeat = { ...(item.repeat ?? {}), sequence: follow(wireFrom(current, "body"), `${path} › boucle`) };
          break;
      }
      list.push(item);
      current = wireFrom(current, "next");
    }
    return list;
  };
  config.actions = follow(key, "");
  if (config.actions.length === 0) errors.push("Ajoute au moins une action (« Alors… »).");

  const loose = nodes.filter((n) => !visited.has(n.key));
  for (const node of loose) node.path = null;
  if (loose.length > 0) {
    errors.push(`${loose.length} bloc(s) ne sont reliés à rien : relie-les ou supprime-les.`);
  }
  return { config, layout, errors, loose: new Set(loose.map((n) => n.key)) };
}

// ----- Affichage

/** La position d'une sortie (`port`) ou de l'entrée (`null`) d'un bloc. */
function portPoint(node, port) {
  if (port === null) return [node.x, node.y + nodeHeight(node) / 2];
  const list = outputs(node);
  const index = Math.max(0, list.findIndex(([name]) => name === port));
  const top = node.y + (nodeHeight(node) - (list.length - 1) * PORT_GAP) / 2;
  return [node.x + NODE_W, top + index * PORT_GAP];
}

function curve([x1, y1], [x2, y2]) {
  const dx = Math.max(48, Math.abs(x2 - x1) / 2);
  return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
}

/** Le statut d'un bloc dans l'exécution affichée : done, failed, stopped… */
function runStatus(node) {
  const run = canvas.run;
  if (!run || !node.path) return null;
  if (node.kind === "trigger") {
    const index = Number(node.path.split(" ")[1]) - 1;
    return run.trigger_index === index ? { status: "done", detail: run.trigger ?? "" } : null;
  }
  if (node.kind === "condition") {
    const index = Number(node.path.split(" ")[1]) - 1;
    const check = run.conditions?.[index];
    return check ? { status: check.passed ? "done" : "stopped", detail: check.passed ? "vraie" : "fausse" } : null;
  }
  // Les tours d'une répétition s'appellent « tour 2 » dans l'historique.
  const steps = (run.steps ?? []).filter((step) => step.path.replace(/tour \d+/g, "boucle") === node.path);
  const step = steps.at(-1);
  if (!step) return null;
  const at = new Date(step.at);
  const time = Number.isNaN(at.getTime()) ? "" : timeOnly.format(at);
  const rounds = steps.length > 1 ? ` · ${steps.length} fois` : "";
  return { status: step.status, detail: [time, step.detail].filter(Boolean).join(" · ") + rounds };
}

function renderCanvas() {
  const root = $("#auto-canvas");
  const { loose, errors } = graphToConfig(auto.config ?? {});
  const stage = $("#canvas-stage");
  stage.style.transform = `translate(${canvas.view.x}px, ${canvas.view.y}px) scale(${canvas.view.scale})`;
  root.classList.toggle("tracing", canvas.run !== null);
  root.classList.toggle("readonly", canvas.readOnly);

  // Les fils.
  const wires = svg("g");
  canvas.wires.forEach((wire, index) => {
    const from = canvas.nodes.get(wire.from);
    const to = canvas.nodes.get(wire.to);
    if (!from || !to) return;
    const d = curve(portPoint(from, wire.port), portPoint(to, null));
    const reached = canvas.run && runStatus(from) && runStatus(to);
    const path = svg("path", { d, class: `wire${index === canvas.selectedWire ? " selected" : ""}${reached ? " reached" : ""}` });
    // Une zone plus large, invisible, pour viser le fil au doigt.
    const hit = svg("path", { d, class: "wire-hit", "data-wire": String(index) });
    wires.append(path, hit);
  });
  const lines = $("#canvas-wires");
  lines.replaceChildren(wires);
  if (canvas.drag?.type === "wire") {
    lines.append(svg("path", { d: curve(canvas.drag.start, canvas.drag.end), class: "wire drafting" }));
  }

  // Les blocs.
  const blocks = [...canvas.nodes.values()].map((node) => {
    const [label, kind] = nodeTitle(node);
    const status = runStatus(node);
    const classes = ["node", `node-${node.kind === "action" ? actionKind(node.item) : node.kind}`];
    if (outputs(node).some(([, text]) => text)) classes.push("node-ports");
    if (node.key === canvas.selected) classes.push("selected");
    if (loose.has(node.key)) classes.push("loose");
    if (status) classes.push(`run-${status.status}`);
    else if (canvas.run) classes.push("run-none");
    const box = el("div", { class: classes.join(" "), "data-node": node.key });
    // La politique de sécurité (CSP) refuse les attributs `style` : les
    // positions passent par l'objet `style`, autorisé.
    Object.assign(box.style, { left: `${node.x}px`, top: `${node.y}px`, width: `${NODE_W}px`, height: `${nodeHeight(node)}px` });
    const body = el(
      "button",
      { type: "button", class: "node-body", "data-node": node.key, "aria-label": `${label} : ${kind}. ${nodeText(node)}` },
      el("span", { class: "node-head" }, el("span", { class: "node-icon" }, svgIcon(nodeIcon(node))), el("span", { class: "node-kind", text: `${label} · ${kind}` })),
      el("span", { class: "node-text", text: nodeText(node) || "à compléter" }),
    );
    box.append(body);
    if (status?.detail) box.append(el("span", { class: "node-run", text: status.detail }));
    if (hasInput(node)) box.append(el("span", { class: "port port-in", "data-in": node.key, title: "Entrée" }));
    const list = outputs(node);
    list.forEach(([port, text]) => {
      const [, y] = portPoint(node, port);
      const top = y - node.y;
      const out = el("span", { class: "port port-out", "data-out": node.key, "data-port": port, title: text || "Sortie" });
      out.style.top = `${top}px`;
      box.append(out);
      if (text) {
        const name = el("span", { class: "port-label", text });
        name.style.top = `${top}px`;
        box.append(name);
      }
      // « + » au bout d'une sortie libre : ajouter un bloc relié, comme n8n.
      if (!canvas.readOnly && !wireFrom(node.key, port)) {
        const add = el("button", { type: "button", class: "port-add", "data-add": node.key, "data-port": port, "aria-label": `Ajouter un bloc après « ${kind} »${text ? ` (${text})` : ""}`, text: "+" });
        add.style.top = `${top}px`;
        box.append(add);
      }
    });
    return box;
  });
  $("#canvas-nodes").replaceChildren(...blocks);
  $("#canvas-errors").replaceChildren(...errors.map((error) => el("li", { text: error })));
  $("#canvas-wire-delete").hidden = canvas.selectedWire === null || canvas.readOnly;
  $("#canvas-add").hidden = canvas.readOnly;
  $("#canvas-tidy").hidden = canvas.readOnly;
  return errors;
}

// ----- Le panneau : les réglages du bloc choisi

function openPanel(key) {
  canvas.selected = key;
  canvas.selectedWire = null;
  const node = canvas.nodes.get(key);
  const panel = $("#canvas-panel");
  if (!node) {
    panel.hidden = true;
    renderCanvas();
    return;
  }
  const [label, kind] = nodeTitle(node);
  const refresh = () => openPanel(key);
  const fields = panelFields(node, refresh);
  const box = el("div", { class: "auto-fields" }, ...fields);
  const remove = el("button", { type: "button", class: "ghost danger small", text: "Supprimer le bloc" });
  remove.addEventListener("click", () => removeNode(key));
  const close = el("button", { type: "button", class: "ghost small", text: "Fermer", "aria-label": "Fermer les réglages du bloc" });
  close.addEventListener("click", () => openPanel(null));
  panel.replaceChildren(
    el("div", { class: "canvas-panel-head" }, el("strong", { text: `${label} · ${kind}` }), close),
    box,
  );
  if (!canvas.readOnly) panel.append(el("div", { class: "auto-actions" }, remove));
  else for (const control of box.querySelectorAll("input, select, button, textarea")) control.disabled = true;
  panel.hidden = false;
  renderCanvas();
}

/** Les champs d'un bloc : ceux du formulaire, sans les suites d'actions
 *  (elles sont dessinées par les fils). */
function panelFields(node, refresh) {
  if (node.kind === "trigger") return triggerFields(node.item);
  if (node.kind === "condition") return conditionFields(node.item);
  const a = node.item;
  switch (actionKind(a)) {
    case "if":
      a.if ??= [];
      return [
        nested(itemsBlock("Si…", "Toutes doivent être vraies. « alors » et « sinon » sont les sorties du bloc.", a, "if", CONDITION_KINDS, "condition", conditionFields)),
      ];
    case "choose": {
      a.choose ??= [];
      const blocks = a.choose.map((option, i) => {
        option.conditions ??= [];
        const block = nested(itemsBlock(`Option ${i + 1}`, "Toutes doivent être vraies.", option, "conditions", CONDITION_KINDS, "condition", conditionFields));
        const remove = el("button", { type: "button", class: "ghost small", text: `Retirer l'option ${i + 1}` });
        remove.addEventListener("click", () => {
          // Les fils des options suivantes remontent d'un cran.
          a.choose.splice(i, 1);
          canvas.wires = canvas.wires
            .filter((w) => !(w.from === node.key && w.port === `opt${i}`))
            .map((w) => {
              const n = w.from === node.key && /^opt\d+$/.test(w.port) ? Number(w.port.slice(3)) : -1;
              return n > i ? { ...w, port: `opt${n - 1}` } : w;
            });
          canvas.structureChanged = true;
          refresh();
          scheduleCheck();
        });
        block.append(remove);
        return block;
      });
      const add = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une option" });
      add.addEventListener("click", () => {
        a.choose.push({ conditions: [] });
        canvas.structureChanged = true;
        refresh();
        scheduleCheck();
      });
      return [...blocks, add];
    }
    case "repeat": {
      a.repeat ??= { count: 3 };
      const r = a.repeat;
      const mode = REPEAT_MODES.find(([key]) => r[key] !== undefined)?.[0] ?? "count";
      const choice = { mode };
      const kind = selectField("Combien de fois", choice, "mode", REPEAT_MODES, false, () => {
        for (const [key] of REPEAT_MODES) delete r[key];
        r[choice.mode] = choice.mode === "count" ? 3 : [];
        refresh();
      });
      const how = mode === "count"
        ? numberField("Nombre de fois (1 à 100)", r, "count")
        : nested(itemsBlock(mode === "while" ? "Tant que…" : "Jusqu'à ce que…", "Toutes doivent être vraies.", r, mode, CONDITION_KINDS, "condition", conditionFields));
      return [kind, how, el("p", { class: "journal-hint", text: "Les actions à répéter partent de la sortie « à répéter »." })];
    }
    default:
      return actionFields(a, refresh);
  }
}

function removeNode(key) {
  canvas.nodes.delete(key);
  canvas.wires = canvas.wires.filter((w) => w.from !== key && w.to !== key);
  canvas.structureChanged = true;
  canvas.selected = null;
  $("#canvas-panel").hidden = true;
  renderCanvas();
  scheduleCheck();
}

// ----- Ajouter un bloc

/** Le menu des blocs à ajouter, à l'endroit `[x, y]` du schéma. Si `from`
 *  est donné, le nouveau bloc est relié à cette sortie. */
function openAddMenu(screenX, screenY, at, from = null) {
  const menu = $("#canvas-menu");
  const groups = [];
  const fromNode = from ? canvas.nodes.get(from.key) : null;
  // Après un déclencheur ou depuis rien : on peut aussi ajouter un déclencheur.
  if (!fromNode) groups.push(["Quand… (déclencheur)", "trigger", TRIGGER_KINDS]);
  groups.push(["Si… (condition)", "condition", CONDITION_KINDS]);
  groups.push(["Alors… (action)", "action", ACTION_KINDS]);
  const list = el("div", { class: "canvas-menu-list", role: "menu" });
  for (const [title, kind, kinds] of groups) {
    list.append(el("p", { class: "canvas-menu-title", text: title }));
    for (const [value, [text, make]] of Object.entries(kinds)) {
      // Une condition « du début » n'existe que juste après les déclencheurs ;
      // ailleurs, c'est « continuer seulement si… » (une action).
      if (kind === "condition" && fromNode && fromNode.kind !== "trigger" && fromNode.kind !== "condition") continue;
      if (kind === "action" && value === "condition" && (!fromNode || fromNode.kind === "trigger")) continue;
      const item = el("button", { type: "button", role: "menuitem", class: "canvas-menu-item", text });
      item.addEventListener("click", () => {
        menu.hidden = true;
        let made = make();
        if (kind === "action") made = stripBranches(made);
        const key = addNode(kind, made, at[0], at[1]);
        if (fromNode) {
          // Ajouté entre deux blocs : le bloc suivant passe après le nouveau.
          const after = wireFrom(fromNode.key, from.port);
          connect(fromNode.key, from.port, key);
          if (after && kind !== "trigger") connect(key, "next", after);
        } else if (kind === "trigger") {
          const start = canvas.nodes.size > 1 ? firstBlock() : null;
          if (start) connect(key, "next", start);
        }
        canvas.structureChanged = true;
        openPanel(key);
        scheduleCheck();
      });
      list.append(item);
    }
  }
  const close = el("button", { type: "button", class: "ghost small", text: "Annuler" });
  close.addEventListener("click", () => {
    menu.hidden = true;
  });
  menu.replaceChildren(list, close);
  menu.hidden = false;
  const area = $("#auto-canvas").getBoundingClientRect();
  const left = Math.min(Math.max(8, screenX - area.left), Math.max(8, area.width - 280));
  const top = Math.min(Math.max(8, screenY - area.top), Math.max(8, area.height - 320));
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;
  list.querySelector("button")?.focus();
}

/** Le bloc où arrivent les déclencheurs. */
function firstBlock() {
  for (const node of canvas.nodes.values()) {
    if (node.kind === "trigger") {
      const to = wireFrom(node.key, "next");
      if (to) return to;
    }
  }
  return null;
}

// ----- Vue : déplacement et zoom

function toCanvas(clientX, clientY) {
  const area = $("#auto-canvas").getBoundingClientRect();
  return [
    (clientX - area.left - canvas.view.x) / canvas.view.scale,
    (clientY - area.top - canvas.view.y) / canvas.view.scale,
  ];
}

function zoomAt(clientX, clientY, factor) {
  const area = $("#auto-canvas").getBoundingClientRect();
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, canvas.view.scale * factor));
  const px = clientX - area.left;
  const py = clientY - area.top;
  canvas.view.x = px - ((px - canvas.view.x) * scale) / canvas.view.scale;
  canvas.view.y = py - ((py - canvas.view.y) * scale) / canvas.view.scale;
  canvas.view.scale = scale;
  $("#canvas-stage").style.transform = `translate(${canvas.view.x}px, ${canvas.view.y}px) scale(${scale})`;
}

/** Tout le schéma dans la fenêtre. */
function fitView() {
  const area = $("#auto-canvas").getBoundingClientRect();
  const nodes = [...canvas.nodes.values()];
  if (nodes.length === 0 || area.width === 0) {
    canvas.view = { x: 40, y: 40, scale: 1 };
    return;
  }
  const minX = Math.min(...nodes.map((n) => n.x));
  const minY = Math.min(...nodes.map((n) => n.y));
  const maxX = Math.max(...nodes.map((n) => n.x + NODE_W + 90));
  const maxY = Math.max(...nodes.map((n) => n.y + nodeHeight(n)));
  const pad = 32;
  const scale = Math.min(1, Math.max(MIN_SCALE, Math.min((area.width - 2 * pad) / (maxX - minX), (area.height - 2 * pad) / (maxY - minY))));
  canvas.view = {
    scale,
    x: pad - minX * scale + Math.max(0, (area.width - 2 * pad - (maxX - minX) * scale) / 2),
    y: pad - minY * scale + Math.max(0, (area.height - 2 * pad - (maxY - minY) * scale) / 2),
  };
}

function scheduleLayoutSave() {
  // La place des blocs n'a de nom stable que si le dessin n'a pas changé.
  if (auto.existingId === null || canvas.structureChanged || !auto.editable) return;
  clearTimeout(canvas.layoutTimer);
  canvas.layoutTimer = setTimeout(() => {
    const { layout, errors } = graphToConfig(auto.config ?? {});
    if (errors.length === 0) saveLayout(auto.existingId, layout);
  }, 800);
}

async function saveLayout(id, layout) {
  const { status } = await api("PUT", `/api/automations/${encodeURIComponent(id)}/layout`, layout);
  const entry = auto.list.find((e) => e.id === id);
  if (status === 204 && entry) entry.layout = layout;
}

function setupCanvasEvents() {
  const area = $("#auto-canvas");

  area.addEventListener("pointerdown", (event) => {
    // Un appui ailleurs que dans le menu le ferme.
    if (!event.target.closest?.(".canvas-menu")) $("#canvas-menu").hidden = true;
    canvas.pointers.set(event.pointerId, [event.clientX, event.clientY]);
    if (canvas.pointers.size === 2) {
      // Deux doigts : on pince pour zoomer (et on annule le geste en cours).
      const [a, b] = [...canvas.pointers.values()];
      canvas.drag = { type: "pinch", distance: Math.hypot(a[0] - b[0], a[1] - b[1]), center: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2] };
      return;
    }
    const target = event.target;
    const out = target.closest?.("[data-out]");
    const node = target.closest?.("[data-node]");
    const hit = target.closest?.("[data-wire]");
    if (target.closest?.(".port-add, .canvas-tools, .canvas-panel, .canvas-menu, .canvas-errors")) return;
    area.setPointerCapture(event.pointerId);
    if (out && !canvas.readOnly) {
      const from = canvas.nodes.get(out.dataset.out);
      const start = portPoint(from, out.dataset.port);
      canvas.drag = { type: "wire", from: from.key, port: out.dataset.port, start, end: start };
    } else if (node) {
      const n = canvas.nodes.get(node.dataset.node);
      const [x, y] = toCanvas(event.clientX, event.clientY);
      canvas.drag = { type: "node", key: n.key, dx: x - n.x, dy: y - n.y, moved: false, sx: event.clientX, sy: event.clientY };
    } else if (hit) {
      canvas.selectedWire = Number(hit.dataset.wire);
      canvas.selected = null;
      $("#canvas-panel").hidden = true;
      renderCanvas();
      canvas.drag = null;
    } else {
      canvas.drag = { type: "pan", sx: event.clientX, sy: event.clientY, vx: canvas.view.x, vy: canvas.view.y, moved: false };
    }
  });

  area.addEventListener("pointermove", (event) => {
    if (!canvas.pointers.has(event.pointerId)) return;
    canvas.pointers.set(event.pointerId, [event.clientX, event.clientY]);
    const drag = canvas.drag;
    if (!drag) return;
    if (drag.type === "pinch" && canvas.pointers.size === 2) {
      const [a, b] = [...canvas.pointers.values()];
      const distance = Math.hypot(a[0] - b[0], a[1] - b[1]);
      const center = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      canvas.view.x += center[0] - drag.center[0];
      canvas.view.y += center[1] - drag.center[1];
      zoomAt(center[0], center[1], distance / drag.distance);
      drag.distance = distance;
      drag.center = center;
    } else if (drag.type === "pan") {
      canvas.view.x = drag.vx + event.clientX - drag.sx;
      canvas.view.y = drag.vy + event.clientY - drag.sy;
      drag.moved ||= Math.hypot(event.clientX - drag.sx, event.clientY - drag.sy) > 4;
      $("#canvas-stage").style.transform = `translate(${canvas.view.x}px, ${canvas.view.y}px) scale(${canvas.view.scale})`;
    } else if (drag.type === "node") {
      if (!drag.moved && Math.hypot(event.clientX - drag.sx, event.clientY - drag.sy) < 5) return;
      drag.moved = true;
      const node = canvas.nodes.get(drag.key);
      const [x, y] = toCanvas(event.clientX, event.clientY);
      // Une grille de 10 px : les blocs s'alignent facilement.
      node.x = Math.round((x - drag.dx) / 10) * 10;
      node.y = Math.round((y - drag.dy) / 10) * 10;
      renderCanvas();
    } else if (drag.type === "wire") {
      drag.end = toCanvas(event.clientX, event.clientY);
      renderCanvas();
    }
  });

  const end = (event) => {
    canvas.pointers.delete(event.pointerId);
    const drag = canvas.drag;
    if (!drag || (drag.type === "pinch" && canvas.pointers.size > 0)) return;
    canvas.drag = null;
    if (drag.type === "node") {
      if (drag.moved) scheduleLayoutSave();
      else openPanel(drag.key);
    } else if (drag.type === "wire") {
      // Relâché sur un bloc : on le relie ; dans le vide : menu « ajouter ».
      const under = document.elementFromPoint(event.clientX, event.clientY);
      const targetKey = under?.closest?.("[data-in]")?.dataset.in ?? under?.closest?.("[data-node]")?.dataset.node;
      if (targetKey && targetKey !== drag.from && hasInput(canvas.nodes.get(targetKey))) {
        connect(drag.from, drag.port, targetKey);
        canvas.structureChanged = true;
        scheduleCheck();
      } else if (!targetKey && Math.hypot(drag.end[0] - drag.start[0], drag.end[1] - drag.start[1]) > 30) {
        openAddMenu(event.clientX, event.clientY, [drag.end[0], drag.end[1] - NODE_MIN_H / 2], { key: drag.from, port: drag.port });
      }
      renderCanvas();
    } else if (drag.type === "pan" && !drag.moved) {
      // Un toucher dans le vide : on ferme le panneau, on oublie le fil choisi.
      canvas.selectedWire = null;
      openPanel(null);
    }
  };
  area.addEventListener("pointerup", end);
  area.addEventListener("pointercancel", end);

  area.addEventListener("wheel", (event) => {
    if (event.target.closest(".canvas-panel, .canvas-menu, .canvas-errors")) return;
    event.preventDefault();
    if (event.ctrlKey || event.metaKey) {
      // Pincement sur un pavé tactile (ou Ctrl + molette) : zoom.
      zoomAt(event.clientX, event.clientY, Math.exp(-event.deltaY / 300));
    } else {
      canvas.view.x -= event.deltaX;
      canvas.view.y -= event.deltaY;
      $("#canvas-stage").style.transform = `translate(${canvas.view.x}px, ${canvas.view.y}px) scale(${canvas.view.scale})`;
    }
  }, { passive: false });

  area.addEventListener("click", (event) => {
    const add = event.target.closest(".port-add");
    if (!add) return;
    const node = canvas.nodes.get(add.dataset.add);
    const [px, py] = portPoint(node, add.dataset.port);
    const rect = add.getBoundingClientRect();
    openAddMenu(rect.right, rect.top, [px + 70, py - NODE_MIN_H / 2], { key: node.key, port: add.dataset.port });
  });

  // Clavier : Entrée ouvre un bloc ; les flèches le déplacent ; Suppr retire
  // le bloc ou le fil choisi.
  area.addEventListener("keydown", (event) => {
    if (event.target.closest("input, select, textarea, .canvas-panel, .canvas-menu")) return;
    const key = event.target.closest?.("[data-node]")?.dataset.node ?? canvas.selected;
    const node = key ? canvas.nodes.get(key) : null;
    if ((event.key === "Delete" || event.key === "Backspace") && !canvas.readOnly) {
      if (canvas.selectedWire !== null) {
        deleteSelectedWire();
        event.preventDefault();
      } else if (node) {
        removeNode(node.key);
        event.preventDefault();
      }
    } else if (node && event.key === "Enter") {
      openPanel(node.key);
      event.preventDefault();
    } else if (node && event.key.startsWith("Arrow")) {
      const step = event.shiftKey ? 50 : 10;
      if (event.key === "ArrowLeft") node.x -= step;
      if (event.key === "ArrowRight") node.x += step;
      if (event.key === "ArrowUp") node.y -= step;
      if (event.key === "ArrowDown") node.y += step;
      renderCanvas();
      area.querySelector(`.node-body[data-node="${node.key}"]`)?.focus();
      scheduleLayoutSave();
      event.preventDefault();
    }
  });

  // Les champs du panneau : le texte du bloc suit ce qu'on tape.
  $("#canvas-panel").addEventListener("input", () => renderCanvas());
  $("#canvas-panel").addEventListener("change", () => renderCanvas());

  $("#canvas-zoom-in").addEventListener("click", () => {
    const r = area.getBoundingClientRect();
    zoomAt(r.left + r.width / 2, r.top + r.height / 2, 1.2);
  });
  $("#canvas-zoom-out").addEventListener("click", () => {
    const r = area.getBoundingClientRect();
    zoomAt(r.left + r.width / 2, r.top + r.height / 2, 1 / 1.2);
  });
  $("#canvas-fit").addEventListener("click", () => {
    fitView();
    renderCanvas();
  });
  $("#canvas-add").addEventListener("click", (event) => {
    const r = area.getBoundingClientRect();
    const [x, y] = toCanvas(r.left + r.width / 2, r.top + r.height / 2);
    openAddMenu(event.clientX, event.clientY, [x - NODE_W / 2, y]);
  });
  $("#canvas-tidy").addEventListener("click", () => {
    // Replacer tous les blocs automatiquement (le dessin ne change pas).
    const { config, errors } = graphToConfig(auto.config ?? {});
    if (errors.length > 0) return setAutoStatus(`Relie d'abord tous les blocs pour les ranger : ${errors[0]}`, "error");
    configToGraph(config, {});
    canvas.structureChanged = true;
    fitView();
    renderCanvas();
    scheduleCheck();
  });
  $("#canvas-wire-delete").addEventListener("click", deleteSelectedWire);
  $("#canvas-run").addEventListener("change", (event) => {
    const index = Number(event.target.value);
    canvas.run = Number.isInteger(index) && index >= 0 ? canvas.runs[index] ?? null : null;
    renderCanvas();
  });
}

function deleteSelectedWire() {
  if (canvas.selectedWire === null) return;
  canvas.wires.splice(canvas.selectedWire, 1);
  canvas.selectedWire = null;
  canvas.structureChanged = true;
  renderCanvas();
  scheduleCheck();
}

// ----- Entrée et sortie du mode schéma

/** Ouvre le schéma de `auto.config` (avec la place enregistrée des blocs). */
async function showCanvas() {
  const entry = auto.list.find((e) => e.id === auto.existingId);
  canvas.readOnly = readOnly();
  canvas.run = null;
  canvas.runs = [];
  canvas.selected = null;
  canvas.selectedWire = null;
  configToGraph(auto.config ?? { triggers: [], actions: [] }, entry?.layout ?? {});
  $("#canvas-panel").hidden = true;
  $("#canvas-menu").hidden = true;
  renderCanvas();
  fitView();
  renderCanvas();
  await loadCanvasRuns();
}

/** Les dernières exécutions, à afficher sur le schéma. */
async function loadCanvasRuns() {
  const select = $("#canvas-run");
  select.replaceChildren(el("option", { value: "", text: "Exécutions : aucune affichée" }));
  select.hidden = true;
  if (auto.existingId === null) return;
  const { status, data } = await api("GET", `/api/automations/${encodeURIComponent(auto.existingId)}/traces`);
  if (status !== 200) return;
  canvas.runs = data.traces ?? [];
  canvas.runs.forEach((run, index) => {
    const when = new Date(run.started);
    const time = Number.isNaN(when.getTime()) ? "" : dateTime.format(when);
    select.append(el("option", { value: String(index), text: `${time} · ${outcomeLabel(run.outcome)}` }));
  });
  select.hidden = canvas.runs.length === 0;
}

/** La configuration dessinée, pour la vérification et l'enregistrement. */
function canvasRequest() {
  const { config, errors } = graphToConfig(auto.config ?? {});
  return { config, errors };
}

/** Après un enregistrement réussi : la place des blocs, sous leurs nouveaux noms. */
async function saveCanvasLayout(id) {
  const { layout, errors } = graphToConfig(auto.config ?? {});
  if (errors.length === 0) await saveLayout(id, layout);
}

setupCanvasEvents();
