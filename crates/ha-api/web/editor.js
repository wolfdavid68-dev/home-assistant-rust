// Tableau de bord — l'éditeur d'automatisations (liste, formulaire, versions, exécutions).
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, dialogs.js, editor.js, account.js, start.js,
// puis canvas.js, house.js, agenda.js). Ils partagent leurs déclarations :
// une fonction d'un fichier peut appeler celle d'un autre, une fois la page
// chargée.
"use strict";

// ---------------------------------------------------------------- Automatisations
// L'éditeur travaille directement sur la configuration JSON renvoyée par le
// serveur : le formulaire n'est qu'une vue de cet objet. Un champ que le
// formulaire ne sait pas afficher (paramètre rare…) est donc conservé tel quel.
// Le serveur vérifie tout, à chaque enregistrement : rien n'est fait confiance
// côté navigateur.
const auto = {
  list: [],
  editable: false,
  // Automatisation en cours d'édition.
  config: null,
  existingId: null, // identifiant si on modifie, null si on crée
  source: "editor",
  mode: "form",
  checkTimer: null,
  checkSeq: 0,
};

const TRIGGER_KINDS = {
  state: ["Un appareil change d'état", () => ({ trigger: "state", entity_id: "" })],
  numeric_state: ["Une valeur passe un seuil", () => ({ trigger: "numeric_state", entity_id: "", below: 18 })],
  time: ["À une heure précise", () => ({ trigger: "time", at: "07:30" })],
  sun: ["Lever ou coucher du soleil", () => ({ trigger: "sun", event: "sunset" })],
  event: ["Un événement (minuteur fini…)", () => ({ trigger: "event", event_type: "timer.finished", event_data: {} })],
  time_pattern: ["Régulièrement (toutes les N minutes)", () => ({ trigger: "time_pattern", minutes: "/15" })],
  calendar: ["Un rendez-vous de l'agenda", () => ({ trigger: "calendar", event: "start", entity_id: "calendar.maison", offset: "-00:30:00" })],
};
// Dans une attente, le soleil n'est pas possible (le serveur le refuse).
const WAIT_TRIGGER_KINDS = Object.fromEntries(Object.entries(TRIGGER_KINDS).filter(([kind]) => kind !== "sun"));
const REPEAT_MODES = [["count", "Un nombre de fois"], ["while", "Tant que… (vérifié avant)"], ["until", "Jusqu'à ce que… (vérifié après)"]];
const CONDITION_KINDS = {
  state: ["Un appareil est dans un état", () => ({ condition: "state", entity_id: "", state: "on" })],
  numeric_state: ["Une valeur est au-dessus / au-dessous", () => ({ condition: "numeric_state", entity_id: "", above: 20 })],
  time: ["Plage horaire, jours", () => ({ condition: "time", after: "18:00", before: "23:00" })],
  sun: ["Il fait jour / nuit", () => ({ condition: "sun", after: "sunset", before: "sunrise" })],
  or: ["Au moins une de ces conditions (ou)", () => ({ condition: "or", conditions: [] })],
  and: ["Toutes ces conditions (et)", () => ({ condition: "and", conditions: [] })],
  not: ["Aucune de ces conditions (non)", () => ({ condition: "not", conditions: [] })],
  trigger: ["Selon le déclencheur", () => ({ condition: "trigger", id: "" })],
  template: ["Un calcul est vrai (modèle)", () => ({ condition: "template", value_template: "{{ states('sensor.temperature_salon') | float(0) > 24 }}" })],
};
const ACTION_KINDS = {
  service: ["Commander un appareil", () => ({ action: "light.turn_on", target: { entity_id: "" } })],
  delay: ["Attendre", () => ({ delay: "5min" })],
  if: ["Si… alors… sinon", () => ({ if: [], then: [] })],
  condition: ["Continuer seulement si…", () => ({ condition: "state", entity_id: "", state: "on" })],
  choose: ["Selon le cas (choose)", () => ({ choose: [] })],
  repeat: ["Répéter", () => ({ repeat: { count: 3, sequence: [] } })],
  wait_for_trigger: ["Attendre un événement", () => ({ wait_for_trigger: [{ trigger: "state", entity_id: "" }], timeout: "5min" })],
};
const SERVICES = {
  light: [["turn_on", "Allumer"], ["turn_off", "Éteindre"], ["toggle", "Inverser"]],
  switch: [["turn_on", "Allumer"], ["turn_off", "Éteindre"], ["toggle", "Inverser"]],
  cover: [["open_cover", "Ouvrir"], ["close_cover", "Fermer"], ["stop_cover", "Arrêter"], ["set_cover_position", "Mettre à une position"]],
  script: [["turn_on", "Lancer"], ["turn_off", "Arrêter"]],
  scene: [["turn_on", "Activer"]],
  notify: [["send_message", "Envoyer une notification"]],
  persistent_notification: [["create", "Afficher sur le tableau de bord"], ["dismiss", "Retirer du tableau de bord"]],
  climate: [["set_preset_mode", "Choisir un préréglage"], ["set_temperature", "Régler la consigne"], ["turn_on", "Allumer"], ["turn_off", "Éteindre"]],
  input_boolean: [["turn_on", "Activer"], ["turn_off", "Désactiver"], ["toggle", "Inverser"]],
  input_number: [["set_value", "Régler la valeur"], ["increment", "Augmenter d'un pas"], ["decrement", "Diminuer d'un pas"]],
  input_select: [["select_option", "Choisir"], ["select_next", "Choix suivant"], ["select_previous", "Choix précédent"], ["select_first", "Premier choix"]],
  input_button: [["press", "Appuyer"]],
  alert: [["turn_off", "Compris (faire taire)"], ["turn_on", "Reprendre les rappels"]],
  timer: [["start", "Démarrer"], ["pause", "Mettre en pause"], ["cancel", "Annuler"], ["finish", "Terminer"]],
  lock: [["lock", "Verrouiller"], ["unlock", "Déverrouiller"], ["open", "Ouvrir la porte"]],
  siren: [["turn_on", "Faire sonner"], ["turn_off", "Arrêter"]],
  valve: [["open_valve", "Ouvrir"], ["close_valve", "Fermer"], ["toggle", "Inverser"]],
  vacuum: [["start", "Lancer le ménage"], ["pause", "Mettre en pause"], ["stop", "Arrêter"], ["return_to_base", "Renvoyer à sa base"]],
  humidifier: [["turn_on", "Allumer"], ["turn_off", "Éteindre"], ["set_humidity", "Régler l'humidité voulue"]],
  water_heater: [["set_operation_mode", "Choisir un mode"], ["set_temperature", "Régler la température"]],
  todo: [["add_item", "Ajouter à la liste"], ["remove_completed_items", "Retirer ce qui est fait"]],
  counter: [["increment", "Augmenter"], ["decrement", "Diminuer"], ["reset", "Remettre à zéro"], ["set_value", "Régler la valeur"]],
  input_text: [["set_value", "Écrire"]],
  input_datetime: [["set_datetime", "Régler"]],
  fan: [["turn_on", "Allumer"], ["turn_off", "Éteindre"], ["toggle", "Inverser"], ["set_percentage", "Régler la vitesse"], ["set_preset_mode", "Choisir un mode"]],
  button: [["press", "Appuyer"]],
  number: [["set_value", "Régler la valeur"]],
  select: [["select_option", "Choisir"]],
  text: [["set_value", "Écrire"]],
  alarm_control_panel: [["alarm_arm_away", "Armer (absence)"], ["alarm_arm_home", "Armer (présence)"], ["alarm_arm_night", "Armer (nuit)"], ["alarm_disarm", "Désarmer"]],
  calendar: [["create_event", "Ajouter un rendez-vous"]],
  media_player: [["media_play", "Lancer la lecture"], ["media_pause", "Mettre en pause"], ["media_stop", "Arrêter"], ["media_next_track", "Morceau suivant"], ["volume_set", "Régler le volume"], ["volume_mute", "Couper ou remettre le son"]],
  tts: [["speak", "Faire une annonce"]],
};
const SUN_EVENTS = [["sunrise", "lever du soleil"], ["sunset", "coucher du soleil"], ["dawn", "aube"], ["dusk", "crépuscule"]];
const WEEKDAYS = [["mon", "lun"], ["tue", "mar"], ["wed", "mer"], ["thu", "jeu"], ["fri", "ven"], ["sat", "sam"], ["sun", "dim"]];

$("#open-automations").addEventListener("click", () => {
  $("#automations").showModal();
  showAutoList();
});
$("#automations-close").addEventListener("click", () => $("#automations").close());

// ----- Liste

async function showAutoList(notice = "") {
  $("#automations").classList.remove("canvas-mode");
  $("#auto-edit-view").hidden = true;
  $("#auto-trace-view").hidden = true;
  $("#auto-list-view").hidden = false;
  const message = $("#auto-notice");
  message.textContent = notice;
  message.classList.remove("error");
  message.hidden = !notice;
  // Les automatisations toutes prêtes (pretes.js, étape 85).
  if (typeof renderPresets === "function") renderPresets().catch(() => {});
  const list = $("#auto-list");
  list.replaceChildren(el("li", { class: "auto-empty", text: "Chargement…" }));
  const { status, data } = await api("GET", "/api/automations");
  if (status !== 200) return list.replaceChildren(el("li", { class: "auto-empty", text: "Automatisations indisponibles." }));
  auto.list = data.automations ?? [];
  auto.editable = data.editable === true;
  $("#auto-readonly").hidden = auto.editable;
  $("#auto-new").hidden = !auto.editable;
  list.replaceChildren(...auto.list.map(autoCard));
  if (auto.list.length === 0) {
    list.replaceChildren(el("li", { class: "auto-empty", text: "Aucune automatisation pour l'instant." }));
  }
}

function autoCard(entry) {
  const badges = el("div", { class: "badges" }, el("span", { class: "badge", text: entry.source === "file" ? "fichier" : "éditeur" }));
  if (entry.source === "editor" && entry.id.startsWith("pret_")) badges.append(el("span", { class: "badge", text: "toute prête" }));
  if (entry.error) badges.append(el("span", { class: "badge bad", text: "invalide" }));
  else if (!entry.enabled) badges.append(el("span", { class: "badge off", text: "désactivée" }));
  const card = el(
    "li",
    { class: "auto-card" },
    el("div", { class: "auto-card-head" }, el("strong", { text: entry.alias }), badges),
    el("p", { class: "auto-summary", text: entry.config ? describe(entry.config) : "" }),
  );
  if (entry.error) card.append(el("p", { class: "error", text: entry.error }));
  if (entry.updated_by && entry.updated_at) {
    card.append(el("small", { class: "auto-meta", text: `Modifiée par ${entry.updated_by} le ${dateTime.format(new Date(entry.updated_at))}` }));
  }
  if (entry.last_run) {
    card.append(el("small", {
      class: `auto-meta run-${entry.last_run.outcome}`,
      text: `Dernière exécution il y a ${ago(entry.last_run.started)} · ${outcomeLabel(entry.last_run.outcome)}`,
    }));
  }
  const buttons = el("div", { class: "auto-card-buttons" });
  const open = el("button", { type: "button", class: "action-button", text: entry.source === "editor" && auto.editable ? "Modifier" : "Voir" });
  open.addEventListener("click", () => openAutoEditor(entry));
  buttons.append(open);
  if (!entry.error) {
    const history = el("button", { type: "button", class: "action-button", text: "Historique" });
    history.addEventListener("click", () => openTraces(entry));
    buttons.append(history);
  }
  if (entry.source === "editor" && auto.editable && entry.config) {
    const toggle = el("button", { type: "button", class: "action-button", text: entry.enabled ? "Désactiver" : "Activer" });
    toggle.addEventListener("click", () => {
      const config = structuredClone(entry.config);
      if (entry.enabled) config.enabled = false;
      else delete config.enabled;
      saveAutomation(entry.id, { config }, entry.enabled ? "Automatisation désactivée." : "Automatisation activée.");
    });
    buttons.append(toggle);
  }
  card.append(buttons);
  return card;
}

/** Résumé en français : « Quand … · Si … · Alors … ». */
function describe(config) {
  const parts = [`Quand ${(config.triggers ?? []).map(describeTrigger).join(", ou ")}`];
  if (config.conditions?.length) parts.push(`si ${config.conditions.map(describeCondition).join(" et ")}`);
  parts.push(`alors ${(config.actions ?? []).map(describeAction).join(", puis ")}`);
  return parts.join(" · ");
}

const asList = (value) => (Array.isArray(value) ? value : value === undefined || value === "" ? [] : [value]);
const entityNames = (value) => asList(value).map((id) => (entities.has(id) ? nameOf(entities.get(id)) : id)).join(" ou ") || "?";
const sunLabel = (event) => SUN_EVENTS.find(([key]) => key === event)?.[1] ?? event;

function describeTrigger(t) {
  switch (t.trigger) {
    case "state":
      return `${entityNames(t.entity_id)} ${t.to !== undefined ? `passe à « ${t.to} »` : "change"}${t.for ? ` depuis ${t.for}` : ""}`;
    case "numeric_state":
      return `${entityNames(t.entity_id)} ${[t.above !== undefined && `dépasse ${t.above}`, t.below !== undefined && `descend sous ${t.below}`].filter(Boolean).join(" et ")}`;
    case "time":
      return `à ${asList(t.at).map((at) => (String(at).includes(".") ? `l'heure de ${entityNames(at)}` : at)).join(", ")}`;
    case "sun":
      return `${offsetText(t.offset)}${sunLabel(t.event)}`;
    case "calendar":
      return `${offsetText(t.offset)}${t.event === "end" ? "fin" : "début"} d'un rendez-vous de ${entityNames(t.entity_id)}`;
    case "event":
      return `événement ${t.event_type}${t.event_data?.entity_id ? ` de ${entityNames(t.event_data.entity_id)}` : ""}`;
    case "time_pattern": {
      const step = typeof t.minutes === "string" && /^\/\d+$/.test(t.minutes) && t.hours === undefined ? t.minutes.slice(1) : null;
      return step ? `toutes les ${step} minutes` : `motif horaire ${t.hours ?? "*"} h ${t.minutes ?? "0"} min`;
    }
    default:
      return String(t.trigger);
  }
}

function describeCondition(c) {
  const list = (items) => (Array.isArray(items) ? items : []).map(describeCondition);
  switch (c.condition) {
    case "or":
      return `(${list(c.conditions).join(" ou ")})`;
    case "and":
      return `(${list(c.conditions).join(" et ")})`;
    case "not":
      return `pas (${list(c.conditions).join(" ou ")})`;
    case "trigger":
      return `déclenchée par « ${asList(c.id).join(" » ou « ")} »`;
    case "template":
      return `${String(c.value_template ?? "").trim()} est vrai`;
    case "state":
      return `${entityNames(c.entity_id)} est « ${asList(c.state).join(" » ou « ")} »`;
    case "numeric_state":
      return `${entityNames(c.entity_id)} ${[c.above !== undefined && `> ${c.above}`, c.below !== undefined && `< ${c.below}`].filter(Boolean).join(" et ")}`;
    case "time": {
      const days = asList(c.weekday);
      return [c.after && `après ${c.after}`, c.before && `avant ${c.before}`, days.length && `le ${days.join(", ")}`].filter(Boolean).join(" ");
    }
    case "sun":
      return [c.after && `après le ${sunLabel(c.after)}`, c.before && `avant le ${sunLabel(c.before)}`].filter(Boolean).join(" et ");
    default:
      return String(c.condition);
  }
}

function describeAction(a) {
  const actions = (items) => (Array.isArray(items) ? items : []).map(describeAction).join(", puis ");
  switch (actionKind(a)) {
    case "delay":
      return `attendre ${a.delay}`;
    case "condition":
      return `seulement si ${describeCondition(a)}`;
    case "if": {
      const conditions = (Array.isArray(a.if) ? a.if : []).map(describeCondition).join(" et ");
      const otherwise = Array.isArray(a.else) && a.else.length ? ` sinon ${actions(a.else)}` : "";
      return `si ${conditions} alors ${actions(a.then)}${otherwise}`;
    }
    case "choose":
      return `selon le cas (${(a.choose ?? []).length} option(s))`;
    case "repeat": {
      const r = a.repeat ?? {};
      const how = r.count !== undefined ? `${r.count} fois`
        : r.while ? `tant que ${(r.while ?? []).map(describeCondition).join(" et ")}`
        : `jusqu'à ce que ${(r.until ?? []).map(describeCondition).join(" et ")}`;
      return `répéter ${how} : ${actions(r.sequence)}`;
    }
    case "wait_for_trigger": {
      const what = (Array.isArray(a.wait_for_trigger) ? a.wait_for_trigger : []).map(describeTrigger).join(" ou ");
      const limit = a.timeout ? ` (au plus ${a.timeout}${a.continue_on_timeout === false ? ", sinon on s'arrête" : ""})` : "";
      return `attendre : ${what}${limit}`;
    }
  }
  const [domain, service] = String(a.action).split(".");
  const label = (lookup(SERVICES, domain) ?? []).find(([key]) => key === service)?.[1];
  return `${(label ?? a.action).toLowerCase()} ${entityNames(a.target?.entity_id)}`;
}

function offsetText(offset) {
  const minutes = offsetMinutes(offset);
  if (!minutes) return "au ";
  return `${Math.abs(minutes)} min ${minutes < 0 ? "avant le " : "après le "}`;
}

// ----- Édition

$("#auto-new").addEventListener("click", () =>
  openAutoEditor({
    id: null,
    source: "editor",
    config: { id: "", alias: "", triggers: [TRIGGER_KINDS.state[1]()], actions: [ACTION_KINDS.service[1]()] },
  }),
);

function openAutoEditor(entry) {
  auto.existingId = entry.id;
  auto.source = entry.source;
  auto.config = structuredClone(entry.config ?? { id: entry.id, triggers: [], actions: [] });
  // Par défaut, le schéma (étape 35) : en blocs reliés, façon n8n.
  auto.mode = "canvas";
  $("#auto-list-view").hidden = true;
  $("#auto-trace-view").hidden = true;
  $("#auto-edit-view").hidden = false;
  $("#auto-version-list").hidden = true;
  $("#auto-file-note").hidden = entry.source !== "file";
  $("#auto-yaml").value = entry.yaml ?? "";
  setAutoStatus("");
  renderAutoEditor();
  if (entry.id === null) $("#auto-canvas-meta input")?.focus();
}

function readOnly() {
  return auto.source === "file" || !auto.editable;
}

function renderAutoEditor() {
  const ro = readOnly();
  $("#auto-tab-canvas").setAttribute("aria-pressed", String(auto.mode === "canvas"));
  $("#auto-tab-form").setAttribute("aria-pressed", String(auto.mode === "form"));
  $("#auto-tab-yaml").setAttribute("aria-pressed", String(auto.mode === "yaml"));
  $("#auto-tab-form").hidden = ro;
  $("#automations").classList.toggle("canvas-mode", auto.mode === "canvas");
  $("#auto-canvas-wrap").hidden = auto.mode !== "canvas";
  $("#auto-form").hidden = auto.mode !== "form";
  $("#auto-yaml-label").hidden = auto.mode !== "yaml";
  $("#auto-yaml").readOnly = ro;
  $("#auto-save").hidden = ro;
  $("#auto-delete").hidden = ro || auto.existingId === null;
  $("#auto-duplicate").hidden = !auto.editable || auto.existingId === null;
  $("#auto-versions").hidden = auto.source === "file";
  if (auto.mode === "form") renderAutoForm();
  if (auto.mode === "canvas") {
    // Nom, identifiant, mode : repliés, sauf pour une nouvelle automatisation
    // (il faut lui donner un nom). Le schéma garde ainsi toute la place.
    const meta = $("#auto-canvas-meta");
    const details = el("details", { class: "canvas-meta" }, el("summary", { text: `${auto.config.alias || auto.config.id || "Nouvelle automatisation"} · nom et réglages` }), ...metaFields(auto.config, true));
    details.open = auto.existingId === null;
    meta.replaceChildren(details);
    if (ro) for (const control of meta.querySelectorAll("input, select")) control.disabled = true;
    showCanvas();
  }
}

/** Quitte le schéma : la configuration dessinée doit être complète. */
function leaveCanvas() {
  if (auto.mode !== "canvas") return true;
  if (readOnly()) return true;
  const { config, errors } = canvasRequest();
  if (errors.length > 0) {
    setAutoStatus(`Pour changer de vue : ${errors.join(" ")}`, "error");
    return false;
  }
  auto.config = config;
  return true;
}

$("#auto-tab-canvas").addEventListener("click", async () => {
  if (auto.mode === "canvas") return;
  if (auto.mode === "yaml" && !readOnly()) {
    const result = await checkAutomation({ yaml: $("#auto-yaml").value });
    if (!result) return;
    auto.config = result.config;
  }
  auto.mode = "canvas";
  renderAutoEditor();
});

$("#auto-tab-form").addEventListener("click", async () => {
  if (auto.mode === "form") return;
  if (auto.mode === "canvas") {
    if (!leaveCanvas()) return;
    auto.mode = "form";
    return renderAutoEditor();
  }
  // YAML → formulaire : le serveur lit le YAML et renvoie le JSON.
  const result = await checkAutomation({ yaml: $("#auto-yaml").value });
  if (!result) return;
  auto.config = result.config;
  auto.mode = "form";
  renderAutoEditor();
});

$("#auto-tab-yaml").addEventListener("click", async () => {
  if (auto.mode === "yaml") return;
  if (readOnly()) {
    auto.mode = "yaml";
    return renderAutoEditor();
  }
  if (!leaveCanvas()) return;
  // Formulaire → YAML : il faut une automatisation valide.
  const result = await checkAutomation({ config: auto.config });
  if (!result) return setAutoStatus("Corrige d'abord l'erreur ci-dessus pour passer en YAML.", "error", true);
  $("#auto-yaml").value = result.yaml;
  auto.mode = "yaml";
  renderAutoEditor();
});

$("#auto-yaml").addEventListener("input", () => scheduleCheck());

function currentRequest() {
  if (auto.mode === "canvas") {
    // Le schéma incomplet (bloc non relié…) n'est pas envoyé au serveur.
    const { config, errors } = canvasRequest();
    return errors.length > 0 ? { invalid: errors } : { config };
  }
  return auto.mode === "yaml" ? { yaml: $("#auto-yaml").value } : { config: auto.config };
}

/** Vérifie auprès du serveur ; affiche le résultat. Renvoie la réponse si valide. */
async function checkAutomation(request = currentRequest()) {
  const seq = ++auto.checkSeq;
  if (request.invalid) {
    setAutoStatus(request.invalid.join(" "), "error");
    return null;
  }
  const { status, data } = await api("POST", "/api/automations/check", request);
  if (seq !== auto.checkSeq) return null; // une vérification plus récente est partie
  if (status !== 200) {
    setAutoStatus(data.error ?? "Automatisation invalide.", "error");
    return null;
  }
  setAutoStatus("✓ Automatisation valide.", "ok", false, data.warnings);
  return data;
}

function scheduleCheck() {
  clearTimeout(auto.checkTimer);
  auto.checkTimer = setTimeout(() => checkAutomation().catch(() => {}), 600);
}

function setAutoStatus(text, kind = "", append = false, warnings = []) {
  const status = $("#auto-status");
  if (append && status.textContent) status.append(el("br"), text);
  else status.textContent = text;
  status.className = `auto-status ${kind}`;
  $("#auto-warnings").replaceChildren(...(warnings ?? []).map((w) => el("li", { text: `⚠ ${w}` })));
}

// ----- Le formulaire

/** Nom, identifiant, activée, et mode : en tête du formulaire et du schéma. */
function metaFields(c, compact = false) {
  const alias = textField("Nom", c, "alias", { placeholder: "Volets au coucher du soleil" });
  alias.querySelector("input").addEventListener("input", () => {
    // À la création, l'identifiant suit le nom.
    if (auto.existingId === null) {
      c.id = slug(c.alias ?? "");
      $("#auto-id").value = c.id;
    }
  });
  const id = textField("Identifiant", c, "id", { placeholder: "volets_coucher_soleil" });
  const idInput = id.querySelector("input");
  idInput.id = "auto-id";
  idInput.disabled = auto.existingId !== null;
  idInput.setAttribute("autocapitalize", "none");
  const enabled = el("input", { type: "checkbox" });
  enabled.checked = c.enabled !== false;
  enabled.addEventListener("change", () => {
    if (enabled.checked) delete c.enabled;
    else c.enabled = false;
    scheduleCheck();
  });

  const mode = el("select");
  for (const [value, text] of [
    ["single", "ignorer le nouveau déclenchement (par défaut)"],
    ["restart", "recommencer depuis le début"],
    ["queued", "attendre la fin, puis recommencer"],
    ["parallel", "en lancer une autre en même temps"],
  ]) mode.append(el("option", { value, text }));
  mode.value = c.mode ?? "single";
  mode.addEventListener("change", () => {
    if (mode.value === "single") delete c.mode;
    else c.mode = mode.value;
    if (mode.value !== "queued" && mode.value !== "parallel") delete c.max;
    scheduleCheck();
  });
  const modeLabel = el("label", {}, el("span", { class: "label", text: compact ? "Si elle se redéclenche" : "Si elle se redéclenche pendant qu'elle tourne encore" }), mode);
  if (compact) return [el("div", { class: "auto-row canvas-meta-row" }, alias, id, modeLabel, el("label", { class: "check" }, enabled, "Activée"))];
  return [el("div", { class: "auto-row" }, alias, id), el("label", { class: "check" }, enabled, "Activée"), modeLabel];
}

function renderAutoForm() {
  // Dans le schéma, les blocs (« + Ajouter… ») redessinent le panneau du bloc.
  if (auto.mode === "canvas") return openPanel(canvas.selected);
  const c = auto.config;
  const form = $("#auto-form");
  const scroll = $("#automations").scrollTop;
  form.replaceChildren(
    ...metaFields(c),
    itemsBlock("Quand…", "Déclencheurs : l'un d'eux suffit.", c, "triggers", TRIGGER_KINDS, "trigger", triggerFields),
    itemsBlock("Si… (facultatif)", "Conditions : toutes doivent être vraies.", c, "conditions", CONDITION_KINDS, "condition", conditionFields),
    itemsBlock("Alors…", "Actions, dans l'ordre.", c, "actions", ACTION_KINDS, null, actionFields),
  );
  $("#automations").scrollTop = scroll;
}

/** Un bloc « Quand / Si / Alors » : la liste, et un bouton pour ajouter. */
function itemsBlock(title, hint, config, key, kinds, tag, fields) {
  const items = config[key] ?? [];
  const block = el("fieldset", { class: "auto-block" }, el("legend", { text: title }), el("p", { class: "journal-hint", text: hint }));
  items.forEach((item, index) => {
    const kind = tag ? item[tag] : actionKind(item);
    const label = lookup(kinds, kind)?.[0] ?? kind;
    const remove = el("button", { type: "button", class: "ghost small", text: "Retirer", "aria-label": `Retirer : ${label}` });
    remove.addEventListener("click", () => {
      items.splice(index, 1);
      // Liste facultative vidée : on retire la clé (comme dans le YAML).
      if (items.length === 0 && (key === "conditions" || key === "else")) delete config[key];
      renderAutoForm();
      scheduleCheck();
    });
    const head = el("div", { class: "auto-item-head" }, el("strong", { text: label }));
    if (["actions", "then", "else", "sequence"].includes(key) && index > 0) {
      const up = el("button", { type: "button", class: "ghost small", text: "↑", "aria-label": "Monter" });
      up.addEventListener("click", () => {
        [items[index - 1], items[index]] = [items[index], items[index - 1]];
        renderAutoForm();
        scheduleCheck();
      });
      head.append(up);
    }
    head.append(remove);
    // Seule cette ligne est redessinée quand un champ change sa forme (autre
    // appareil, autre commande) : les autres gardent le focus et le curseur.
    const box = el("div", { class: "auto-fields" });
    let busy = false;
    const refresh = () => {
      // Retirer le champ qui a le focus déclenche son événement « change »,
      // qui redemanderait un dessin : on l'ignore pendant celui-ci.
      if (busy) return;
      busy = true;
      const focusables = () => [...box.querySelectorAll("input, select")];
      const focused = focusables().indexOf(document.activeElement);
      box.replaceChildren(...fields(item, refresh));
      if (focused >= 0) focusables()[focused]?.focus();
      busy = false;
    };
    refresh();
    block.append(el("div", { class: "auto-item" }, head, box));
  });
  const add = el("select", { "aria-label": `Ajouter (${title})` }, el("option", { value: "", text: "+ Ajouter…" }));
  for (const [value, [text]] of Object.entries(kinds)) {
    // « Selon le cas » se crée dans l'onglet YAML (le formulaire l'affiche seulement).
    if (value !== "choose") add.append(el("option", { value, text }));
  }
  add.addEventListener("change", () => {
    const make = lookup(kinds, add.value)?.[1];
    if (!make) return;
    config[key] = [...items, make()];
    renderAutoForm();
    scheduleCheck();
  });
  block.append(add);
  return block;
}

/** La sorte d'une action, d'après sa clé. */
function actionKind(action) {
  for (const key of ["delay", "if", "choose", "repeat", "wait_for_trigger", "condition"]) {
    if (action[key] !== undefined) return key;
  }
  return "service";
}

function triggerFields(t) {
  return [...triggerKindFields(t), textField("Identifiant (facultatif)", t, "id", { placeholder: "arrivee" })];
}

function triggerKindFields(t) {
  switch (t.trigger) {
    case "state":
      return [
        entityField(t, "entity_id"),
        textField("Attribut (facultatif)", t, "attribute", { placeholder: "ex. : brightness" }),
        textField("De (facultatif)", t, "from", { placeholder: "ex. : off", list: true }),
        textField("À (facultatif)", t, "to", { placeholder: "ex. : Orange, Rouge", list: true }),
        durationField("Depuis au moins (facultatif)", t, "for", true),
      ];
    case "numeric_state":
      return [entityField(t, "entity_id"), numberField("Au-dessus de", t, "above"), numberField("Au-dessous de", t, "below")];
    case "time":
      return [timeField("Heure", t, "at")];
    case "sun":
      return [selectField("Événement", t, "event", SUN_EVENTS), offsetField(t, "offset")];
    case "calendar":
      return [entityField(t, "entity_id"), selectField("Moment", t, "event", [["start", "début du rendez-vous"], ["end", "fin du rendez-vous"]]), offsetField(t, "offset")];
    case "time_pattern":
      return [
        textField("Heures (facultatif)", t, "hours", { placeholder: "* , /2 ou 7" }),
        textField("Minutes", t, "minutes", { placeholder: "/15 ou 30" }),
        textField("Seconde (facultatif)", t, "seconds", { placeholder: "0" }),
      ];
    case "event": {
      t.event_data ??= {};
      const entity = entityField(t.event_data, "entity_id");
      // Vide : n'importe quel appareil (et non « l'appareil sans nom »).
      entity.querySelector("input").addEventListener("input", () => {
        if (t.event_data.entity_id === "") delete t.event_data.entity_id;
      });
      return [textField("Événement", t, "event_type", { placeholder: "timer.finished" }), entity];
    }
    default:
      return [];
  }
}

/** Un bloc imbriqué (dans « ou », « si »…) occupe toute la largeur. */
function nested(block) {
  block.classList.add("nested");
  return block;
}

function conditionFields(c) {
  switch (c.condition) {
    case "or":
    case "and":
    case "not":
      c.conditions ??= [];
      return [nested(itemsBlock("Conditions", lookup(CONDITION_KINDS, c.condition)?.[0] ?? "", c, "conditions", CONDITION_KINDS, "condition", conditionFields))];
    case "trigger":
      return [textField("Identifiant du déclencheur", c, "id", { placeholder: "arrivee", list: true })];
    case "template":
      return [textField("Modèle (vrai ou faux)", c, "value_template", { placeholder: "{{ states('sensor.x') | float(0) > 20 }}" })];
    case "state":
      return [
        entityField(c, "entity_id"),
        textField("Attribut (facultatif)", c, "attribute", { placeholder: "ex. : niveau_actuel" }),
        textField("État", c, "state", { placeholder: "ex. : on", list: true }),
      ];
    case "numeric_state":
      return [entityField(c, "entity_id"), numberField("Au-dessus de", c, "above"), numberField("Au-dessous de", c, "below")];
    case "time":
      return [timeField("Après (facultatif)", c, "after", true), timeField("Avant (facultatif)", c, "before", true), weekdayField(c)];
    case "sun":
      return [
        selectField("Après", c, "after", SUN_EVENTS, true),
        offsetField(c, "after_offset"),
        selectField("Avant", c, "before", SUN_EVENTS, true),
        offsetField(c, "before_offset"),
      ];
    default:
      return [];
  }
}

function actionFields(a, refresh) {
  switch (actionKind(a)) {
    case "delay":
      return [durationField("Durée", a, "delay", false)];
    case "condition":
      return conditionFields(a);
    case "if":
      a.if ??= [];
      a.then ??= [];
      return [
        nested(itemsBlock("Si…", "Toutes doivent être vraies.", a, "if", CONDITION_KINDS, "condition", conditionFields)),
        nested(itemsBlock("Alors…", "Si c'est vrai.", a, "then", ACTION_KINDS, null, actionFields)),
        nested(itemsBlock("Sinon… (facultatif)", "Si c'est faux.", a, "else", ACTION_KINDS, null, actionFields)),
      ];
    case "choose":
      return [el("p", { class: "journal-hint", text: `Bloc « selon le cas » (${(a.choose ?? []).length} option(s)) : il se modifie dans l'onglet YAML.` })];
    case "repeat": {
      a.repeat ??= { count: 3, sequence: [] };
      const r = a.repeat;
      r.sequence ??= [];
      const mode = REPEAT_MODES.find(([key]) => r[key] !== undefined)?.[0] ?? "count";
      // Changer de sorte de répétition : on garde les actions, on remplace le reste.
      const choice = { mode };
      const kind = selectField("Combien de fois", choice, "mode", REPEAT_MODES, false, () => {
        for (const [key] of REPEAT_MODES) delete r[key];
        r[choice.mode] = choice.mode === "count" ? 3 : [];
        refresh();
      });
      const how = mode === "count"
        ? numberField(`Nombre de fois (1 à 100)`, r, "count")
        : nested(itemsBlock(mode === "while" ? "Tant que…" : "Jusqu'à ce que…", "Toutes doivent être vraies.", r, mode, CONDITION_KINDS, "condition", conditionFields));
      return [kind, how, nested(itemsBlock("Actions à répéter", "Dans l'ordre, à chaque tour.", r, "sequence", ACTION_KINDS, null, actionFields))];
    }
    case "wait_for_trigger":
      a.wait_for_trigger ??= [];
      return [
        nested(itemsBlock("Attendre que…", "L'un d'eux suffit.", a, "wait_for_trigger", WAIT_TRIGGER_KINDS, "trigger", triggerKindFields)),
        durationField("Au plus (facultatif)", a, "timeout", true),
        selectField("Si l'attente est dépassée", a, "continue_on_timeout", [[true, "continuer quand même"], [false, "s'arrêter là"]], true),
      ];
  }
  a.target ??= { entity_id: "" };
  const entity = entityField(a.target, "entity_id", () => {
    // L'appareil change de type : on propose sa première commande.
    const domain = asList(a.target.entity_id)[0]?.split(".")[0];
    const services = lookup(SERVICES, domain);
    if (services && !String(a.action).startsWith(`${domain}.`)) {
      a.action = `${domain}.${services[0][0]}`;
      refresh();
      scheduleCheck();
    }
  });
  const domain = asList(a.target.entity_id)[0]?.split(".")[0] ?? String(a.action).split(".")[0];
  const options = (lookup(SERVICES, domain) ?? []).map(([service, label]) => [`${domain}.${service}`, label]);
  if (!options.some(([value]) => value === a.action)) options.push([a.action, a.action]);
  const fields = [entity, selectField("Commande", a, "action", options, false, refresh)];

  a.data ??= {};
  const extra = { ...a.data };
  if (a.action === "light.turn_on") {
    fields.push(percentField("Luminosité % (facultatif)", a.data, "brightness"));
    delete extra.brightness;
  } else if (a.action === "cover.set_cover_position") {
    fields.push(numberField("Position (0 = fermé, 100 = ouvert)", a.data, "position"));
    delete extra.position;
  } else if (a.action === "climate.set_temperature") {
    fields.push(numberField("Consigne (°C)", a.data, "temperature"));
    delete extra.temperature;
  } else if (a.action === "climate.set_preset_mode") {
    fields.push(textField("Préréglage", a.data, "preset_mode", { placeholder: "eco" }));
    delete extra.preset_mode;
  } else if (a.action === "fan.set_percentage") {
    fields.push(numberField("Vitesse (%)", a.data, "percentage"));
    delete extra.percentage;
  } else if (a.action === "fan.set_preset_mode") {
    fields.push(textField("Mode", a.data, "preset_mode"));
    delete extra.preset_mode;
  } else if (a.action === "counter.set_value") {
    fields.push(numberField("Valeur", a.data, "value"));
    delete extra.value;
  } else if (a.action === "input_text.set_value") {
    fields.push(textField("Texte", a.data, "value"));
    delete extra.value;
  } else if (a.action === "input_datetime.set_datetime") {
    fields.push(textField("Heure (facultatif)", a.data, "time", { placeholder: "07:30" }), textField("Date (facultatif)", a.data, "date", { placeholder: "2026-10-20" }));
    delete extra.time;
    delete extra.date;
  } else if (a.action === "number.set_value") {
    fields.push(numberField("Valeur", a.data, "value"));
    delete extra.value;
  } else if (a.action === "select.select_option") {
    fields.push(textField("Choix", a.data, "option"));
    delete extra.option;
  } else if (a.action === "text.set_value") {
    fields.push(textField("Texte", a.data, "value"));
    delete extra.value;
  } else if (a.action === "todo.add_item") {
    fields.push(textField("Élément", a.data, "item", { placeholder: "Lait" }));
    delete extra.item;
  } else if (a.action === "calendar.create_event") {
    fields.push(
      textField("Titre", a.data, "summary"),
      textField("Début", a.data, "start_date_time", { placeholder: "2026-10-20 18:30" }),
      textField("Fin (facultatif)", a.data, "end_date_time", { placeholder: "2026-10-20 19:30" }),
    );
    delete extra.summary;
    delete extra.start_date_time;
    delete extra.end_date_time;
  } else if (a.action === "humidifier.set_humidity") {
    fields.push(numberField("Humidité voulue (%)", a.data, "humidity"));
    delete extra.humidity;
  } else if (a.action === "water_heater.set_temperature") {
    fields.push(numberField("Température de l'eau (°C)", a.data, "temperature"));
    delete extra.temperature;
  } else if (a.action === "water_heater.set_operation_mode") {
    fields.push(textField("Mode", a.data, "operation_mode", { placeholder: "eco" }));
    delete extra.operation_mode;
  } else if (a.action === "input_number.set_value") {
    fields.push(numberField("Valeur", a.data, "value"));
    delete extra.value;
  } else if (a.action === "input_select.select_option") {
    fields.push(textField("Choix", a.data, "option"));
    delete extra.option;
  } else if (a.action === "timer.start") {
    fields.push(durationField("Durée (facultatif, sinon celle du fichier)", a.data, "duration", true));
    delete extra.duration;
  } else if (a.action === "persistent_notification.create") {
    fields.push(textField("Titre (facultatif)", a.data, "title"), textField("Message", a.data, "message", { placeholder: MESSAGE_EXAMPLE }));
    delete extra.title;
    delete extra.message;
  } else if (a.action === "media_player.volume_set") {
    // Volume en % à l'écran, 0 à 1 dans la configuration (comme Home Assistant).
    const shown = typeof a.data.volume_level === "number" ? Math.round(a.data.volume_level * 100) : "";
    const input = el("input", { type: "number", min: "0", max: "100", inputmode: "numeric", value: shown });
    input.addEventListener("input", () => {
      const percent = Number(input.value);
      if (input.value === "" || !Number.isFinite(percent)) delete a.data.volume_level;
      else a.data.volume_level = Math.min(100, Math.max(0, percent)) / 100;
      changed();
    });
    fields.push(field("Volume (%)", input));
    delete extra.volume_level;
  } else if (a.action === "media_player.volume_mute") {
    fields.push(selectField("Son", a.data, "is_volume_muted", [[true, "Couper"], [false, "Remettre"]]));
    delete extra.is_volume_muted;
  } else if (a.action === "tts.speak") {
    fields.push(
      textField("Enceinte(s)", a.data, "media_player_entity_id", { placeholder: "media_player.salon, media_player.cuisine", list: true }),
      textField("Message", a.data, "message", { placeholder: "Le linge est fini. {{ states('sensor.lave_linge') }}" }),
      numberField("Volume % (facultatif)", a.data, "volume"),
      selectField("Carillon avant", a.data, "carillon", [[true, "Oui"], [false, "Non"]], true),
    );
    delete extra.media_player_entity_id;
    delete extra.message;
    delete extra.volume;
    delete extra.carillon;
  } else if (a.action === "notify.send_message") {
    fields.push(textField("Titre (facultatif)", a.data, "title"), textField("Message", a.data, "message", { placeholder: MESSAGE_EXAMPLE }));
    fields.push(selectField("Priorité", a.data, "priority", [[1, "1 (discrète)"], [2, "2"], [3, "3 (normale)"], [4, "4"], [5, "5 (urgente)"]], true));
    delete extra.title;
    delete extra.message;
    delete extra.priority;
  }
  if (Object.keys(extra).length > 0) {
    fields.push(el("p", { class: "journal-hint", text: `Autres paramètres, conservés : ${JSON.stringify(extra)}` }));
  }
  return fields;
}

// Un message peut contenir des valeurs (modèles, voir docs/modeles.md).
const MESSAGE_EXAMPLE = "ex. : Il fait {{ states('sensor.meteo_temperature') }} °C";

// ----- Champs du formulaire (chacun lit et modifie une clé de l'objet)

function field(label, input) {
  return el("label", {}, el("span", { class: "label", text: label }), input);
}

function changed() {
  scheduleCheck();
}

/** Texte. `list` : plusieurs valeurs séparées par des virgules. Vide = clé retirée. */
function textField(label, object, key, { placeholder = "", list = false } = {}) {
  const value = object[key];
  const input = el("input", { placeholder, value: Array.isArray(value) ? value.join(", ") : value ?? "" });
  input.addEventListener("input", () => {
    const parts = list ? input.value.split(",").map((s) => s.trim()).filter(Boolean) : [input.value];
    if (list ? parts.length === 0 : input.value === "") delete object[key];
    else object[key] = list && parts.length > 1 ? parts : parts[0];
    changed();
  });
  return field(label, input);
}

/** Appareil(s) : suggestions tirées des appareils connus. */
function entityField(object, key, onChange) {
  const value = object[key];
  const input = el("input", {
    list: "entity-list", placeholder: "light.salon", autocapitalize: "none", spellcheck: "false",
    value: Array.isArray(value) ? value.join(", ") : value ?? "",
  });
  input.addEventListener("input", () => {
    const parts = input.value.split(",").map((s) => s.trim()).filter(Boolean);
    object[key] = parts.length > 1 ? parts : parts[0] ?? "";
    changed();
  });
  if (onChange) input.addEventListener("change", onChange);
  const names = asList(value).filter((id) => entities.has(id)).map((id) => nameOf(entities.get(id)));
  input.title = names.join(", ");
  const label = names.length ? `Appareil(s) · ${names.join(", ")}` : "Appareil(s)";
  return field(label, input);
}

function numberField(label, object, key) {
  const input = el("input", { type: "number", step: "any", inputmode: "decimal", value: object[key] ?? "" });
  input.addEventListener("input", () => {
    if (input.value === "" || !Number.isFinite(Number(input.value))) delete object[key];
    else object[key] = Number(input.value);
    changed();
  });
  return field(label, input);
}

/** Luminosité en % à l'écran, 1 à 255 dans la configuration. */
function percentField(label, object, key) {
  const value = typeof object[key] === "number" ? Math.round((object[key] / 255) * 100) : "";
  const input = el("input", { type: "number", min: "1", max: "100", inputmode: "numeric", value });
  input.addEventListener("input", () => {
    const percent = Number(input.value);
    if (input.value === "" || !Number.isFinite(percent)) delete object[key];
    else object[key] = Math.min(255, Math.max(1, Math.round(percent * 2.55)));
    changed();
  });
  return field(label, input);
}

function selectField(label, object, key, options, optional = false, onChange) {
  const select = el("select");
  if (optional) select.append(el("option", { value: "", text: "—" }));
  for (const [value, text] of options) select.append(el("option", { value: String(value), text }));
  select.value = object[key] === undefined ? "" : String(object[key]);
  select.addEventListener("change", () => {
    const option = options.find(([value]) => String(value) === select.value);
    if (!option) delete object[key];
    else object[key] = option[0];
    changed();
    onChange?.();
  });
  return field(label, select);
}

/** Heure : un sélecteur d'heure, ou du texte pour plusieurs heures. */
function timeField(label, object, key, optional = false) {
  const value = object[key];
  const simple = value === undefined || (typeof value === "string" && /^\d{2}:\d{2}$/.test(value));
  if (!simple) return textField(label, object, key, { placeholder: "07:30, 19:00", list: true });
  const input = el("input", { type: "time", value: value ?? "" });
  input.addEventListener("input", () => {
    if (input.value === "" && optional) delete object[key];
    else object[key] = input.value;
    changed();
  });
  return field(label, input);
}

/** Durée : un nombre et une unité (`5min`, `30s`, `2h`). */
function durationField(label, object, key, optional) {
  const seconds = durationSeconds(object[key]);
  const [amount, unit] =
    seconds === null ? ["", "min"] : seconds % 3600 === 0 && seconds > 0 ? [seconds / 3600, "h"] : seconds % 60 === 0 ? [seconds / 60, "min"] : [seconds, "s"];
  const number = el("input", { type: "number", min: "0", inputmode: "numeric", value: String(amount), "aria-label": label });
  const select = el("select", { "aria-label": `Unité : ${label}` });
  for (const [value, text] of [["s", "secondes"], ["min", "minutes"], ["h", "heures"]]) select.append(el("option", { value, text }));
  select.value = unit;
  const update = () => {
    if (number.value === "" && optional) delete object[key];
    else object[key] = `${Math.max(0, Math.round(Number(number.value) || 0))}${select.value}`;
    changed();
  };
  number.addEventListener("input", update);
  select.addEventListener("change", update);
  return el("label", {}, el("span", { class: "label", text: label }), el("span", { class: "inline" }, number, select));
}

/** `5min` → 300 ; `00:05:00` → 300 ; illisible → null. */
function durationSeconds(value) {
  if (typeof value !== "string") return null;
  let match = /^(\d+)(s|min|h)$/.exec(value);
  if (match) return Number(match[1]) * { s: 1, min: 60, h: 3600 }[match[2]];
  match = /^(\d{1,2}):(\d{2})(?::(\d{2}))?$/.exec(value);
  if (match) return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3] ?? 0);
  return null;
}

/** Décalage du soleil, en minutes : `-00:30:00` ↔ 30 minutes avant. */
function offsetMinutes(value) {
  if (typeof value !== "string") return 0;
  const negative = value.startsWith("-");
  const seconds = durationSeconds(value.replace(/^[-+]/, ""));
  return seconds === null ? 0 : Math.round(seconds / 60) * (negative ? -1 : 1);
}

function offsetField(object, key) {
  const minutes = offsetMinutes(object[key]);
  const number = el("input", { type: "number", min: "0", max: "1440", inputmode: "numeric", value: minutes ? String(Math.abs(minutes)) : "", "aria-label": "Décalage en minutes" });
  const select = el("select", { "aria-label": "Avant ou après" });
  for (const [value, text] of [["after", "après"], ["before", "avant"]]) select.append(el("option", { value, text }));
  select.value = minutes < 0 ? "before" : "after";
  const update = () => {
    const total = Math.min(1440, Math.round(Math.abs(Number(number.value) || 0)));
    if (total === 0) delete object[key];
    else {
      const hh = String(Math.floor(total / 60)).padStart(2, "0");
      const mm = String(total % 60).padStart(2, "0");
      object[key] = `${select.value === "before" ? "-" : ""}${hh}:${mm}:00`;
    }
    changed();
  };
  number.addEventListener("input", update);
  select.addEventListener("change", update);
  return el("label", {}, el("span", { class: "label", text: "Décalage en min (facultatif)" }), el("span", { class: "inline" }, number, select));
}

function weekdayField(c) {
  const days = new Set(asList(c.weekday));
  const box = el("div", { class: "weekdays", role: "group", "aria-label": "Jours" });
  for (const [en, fr] of WEEKDAYS) {
    const input = el("input", { type: "checkbox" });
    input.checked = days.has(en) || days.has(fr);
    input.addEventListener("change", () => {
      const boxes = box.querySelectorAll("input");
      const chosen = WEEKDAYS.filter((_, i) => boxes[i].checked).map(([en]) => en);
      if (chosen.length === 0) delete c.weekday;
      else c.weekday = chosen;
      changed();
    });
    box.append(el("label", { class: "check" }, input, fr));
  }
  return el("div", { class: "weekdays-field" }, el("span", { class: "label", text: "Jours (aucun = tous)" }), box);
}

/** « Volets au coucher ! » → « volets_au_coucher » (identifiant valide). */
function slug(text) {
  return text
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 64);
}

// ----- Enregistrer, supprimer, versions

$("#auto-save").addEventListener("click", () => {
  const request = currentRequest();
  if (request.invalid) return setAutoStatus(request.invalid.join(" "), "error");
  const id = auto.mode === "yaml" ? auto.existingId ?? yamlId($("#auto-yaml").value) : auto.config.id;
  if (!id) return setAutoStatus("Donne un nom (ou un identifiant) à l'automatisation.", "error");
  saveAutomation(id, request, auto.existingId === null ? "Automatisation créée." : "Automatisation enregistrée.");
});

/** L'identifiant écrit dans un YAML (`id: …`), pour l'adresse de la requête. */
function yamlId(yaml) {
  return /^\s*-?\s*id:\s*["']?([a-z0-9_]+)["']?\s*$/m.exec(yaml)?.[1] ?? null;
}

async function saveAutomation(id, request, done) {
  const { status, data } = await api("PUT", `/api/automations/${encodeURIComponent(id)}`, request);
  if (status === 403 && data.confirm) return askConfirmation(() => saveAutomation(id, request, done));
  if (status !== 200 && status !== 201) {
    if (!$("#auto-edit-view").hidden) return setAutoStatus(data.error ?? "Enregistrement impossible.", "error");
    return showAutoList(data.error ?? "Enregistrement impossible.");
  }
  const warnings = data.warnings?.length ? ` Attention : ${data.warnings.join(" ; ")}.` : "";
  // Depuis le schéma : la place des blocs, sous leurs noms définitifs.
  if (auto.mode === "canvas" && !$("#auto-edit-view").hidden) await saveCanvasLayout(id).catch(() => {});
  showAutoList(done + warnings);
}

$("#auto-delete").addEventListener("click", () => {
  const id = auto.existingId;
  if (id === null || !window.confirm(`Supprimer l'automatisation « ${auto.config?.alias || id} » ? (Ses versions sont gardées.)`)) return;
  deleteAutomation(id);
});

async function deleteAutomation(id) {
  const { status, data } = await api("DELETE", `/api/automations/${encodeURIComponent(id)}`);
  if (status === 403 && data.confirm) return askConfirmation(() => deleteAutomation(id));
  if (status !== 204) return setAutoStatus(data.error ?? "Suppression impossible.", "error");
  showAutoList("Automatisation supprimée.");
}

$("#auto-duplicate").addEventListener("click", async () => {
  // Depuis le YAML, on repasse par le serveur pour obtenir le JSON.
  if (!leaveCanvas()) return;
  let config = auto.config;
  if (auto.mode === "yaml") {
    const result = await checkAutomation({ yaml: $("#auto-yaml").value }).catch(() => null);
    if (!result) return;
    config = result.config;
  }
  const copy = structuredClone(config);
  const ids = new Set(auto.list.map((entry) => entry.id));
  let id = `${copy.id}_copie`.slice(0, 64);
  for (let n = 2; ids.has(id); n++) id = `${copy.id}_copie_${n}`.slice(0, 64);
  copy.id = id;
  copy.alias = `${copy.alias ?? config.id} (copie)`;
  openAutoEditor({ id: null, source: "editor", config: copy });
  setAutoStatus("Copie prête : modifie-la puis enregistre.", "ok");
});

$("#auto-versions").addEventListener("click", async () => {
  const id = auto.existingId ?? auto.config?.id;
  const list = $("#auto-version-list");
  if (!id) return;
  const { status, data } = await api("GET", `/api/automations/${encodeURIComponent(id)}/versions`);
  if (status !== 200) return setAutoStatus("Versions indisponibles.", "error");
  list.hidden = false;
  list.replaceChildren(
    ...data.versions.map((version) => {
      const item = el(
        "li",
        {},
        el("time", { datetime: version.changed_at, text: dateTime.format(new Date(version.changed_at)) }),
        el("span", { text: `${version.changed_by}${version.deleted ? " · supprimée" : ""}` }),
      );
      if (version.config && !readOnly()) {
        const load = el("button", { type: "button", class: "ghost small", text: "Charger" });
        load.addEventListener("click", () => {
          auto.config = version.config;
          $("#auto-yaml").value = version.yaml ?? "";
          renderAutoEditor();
          setAutoStatus("Version chargée : enregistre pour la rétablir.", "ok");
        });
        item.append(load);
      }
      return item;
    }),
  );
  if (data.versions.length === 0) list.replaceChildren(el("li", { text: "Aucune version enregistrée." }));
});

$("#auto-cancel").addEventListener("click", () => showAutoList());

// ----- Historique des exécutions

const OUTCOMES = {
  running: "en cours",
  finished: "terminée",
  conditions_failed: "conditions non remplies",
  stopped: "arrêtée par une condition",
  already_running: "ignorée : déjà en cours",
  too_many: "ignorée : trop d'exécutions",
  loop: "ignorée : boucle",
  aborted: "interrompue",
};
const outcomeLabel = (outcome) => lookup(OUTCOMES, outcome) ?? String(outcome);
const timeOnly = new Intl.DateTimeFormat("fr", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const trace = { entry: null, runs: [], selected: 0 };

async function openTraces(entry) {
  $("#automations").classList.remove("canvas-mode");
  trace.entry = entry;
  trace.selected = 0;
  $("#auto-list-view").hidden = true;
  $("#auto-edit-view").hidden = true;
  $("#auto-trace-view").hidden = false;
  $("#trace-title").textContent = entry.alias;
  await loadTraces();
}

async function loadTraces() {
  const detail = $("#trace-detail");
  const { status, data } = await api("GET", `/api/automations/${encodeURIComponent(trace.entry.id)}/traces`);
  if (status !== 200) {
    $("#trace-runs").replaceChildren();
    return detail.replaceChildren(el("p", { class: "auto-empty", text: "Historique indisponible." }));
  }
  trace.runs = data.traces ?? [];
  trace.selected = Math.min(trace.selected, Math.max(0, trace.runs.length - 1));
  renderTraces();
}

function renderTraces() {
  const runs = $("#trace-runs");
  if (trace.runs.length === 0) {
    runs.replaceChildren();
    return $("#trace-detail").replaceChildren(
      el("p", { class: "auto-empty", text: "Pas encore d'exécution depuis le démarrage du serveur." }),
    );
  }
  runs.replaceChildren(
    ...trace.runs.map((run, index) => {
      const button = el(
        "button",
        { type: "button", class: `trace-run run-${run.outcome}`, "aria-pressed": String(index === trace.selected) },
        el("span", { class: "trace-dot", "aria-hidden": "true" }),
        el("span", { text: timeOnly.format(new Date(run.started)) }),
      );
      button.title = outcomeLabel(run.outcome);
      button.addEventListener("click", () => {
        trace.selected = index;
        renderTraces();
      });
      return button;
    }),
  );
  $("#trace-detail").replaceChildren(traceDetail(trace.runs[trace.selected]));
}

/** Une durée courte : « 0,4 s », « 12 s », « 3 min 05 s ». */
function shortDuration(ms) {
  if (ms < 10_000) return `${(ms / 1000).toLocaleString("fr", { maximumFractionDigits: 1 })} s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds} s`;
  return `${Math.floor(seconds / 60)} min ${String(seconds % 60).padStart(2, "0")} s`;
}

function traceDetail(run) {
  const started = new Date(run.started);
  const length = run.finished ? ` · ${shortDuration(Date.parse(run.finished) - started.getTime())}` : "";
  const summary = el(
    "div",
    { class: "trace-summary" },
    el("span", { class: `badge trace-outcome run-${run.outcome}`, text: outcomeLabel(run.outcome) }),
    el("small", { text: `${dateTime.format(started)}${length}` }),
  );
  const steps = el("ol", { class: "trace" });
  const step = (kind, title, detail, extra = "") =>
    el("li", { class: `trace-step ${kind}` },
      el("span", { class: "trace-mark", "aria-hidden": "true" }),
      el("div", {}, el("p", { text: title }), ...(detail ? [el("small", { text: detail })] : []), ...(extra ? [el("small", { class: "trace-path", text: extra })] : [])),
    );
  steps.append(step("trigger", "Déclenchée", run.trigger ?? "lancée à la main"));
  for (const check of run.conditions) {
    steps.append(step(check.passed ? "done" : "stopped", `Condition : ${check.description}`, check.passed ? "vraie" : "fausse : rien n'est fait"));
  }
  for (const s of run.steps) {
    const at = timeOnly.format(new Date(s.at));
    const status = s.status === "failed" ? "refusée" : "";
    steps.append(step(s.status, s.description, [status, s.detail].filter(Boolean).join(" · "), `Action ${s.path} · ${at}`));
  }
  if (run.truncated) steps.append(step("stopped", "…", "La suite n'a pas été enregistrée (trop d'étapes)."));
  if (["already_running", "too_many", "loop"].includes(run.outcome)) {
    steps.append(step("stopped", "Pas lancée", outcomeLabel(run.outcome)));
  }
  if (run.outcome === "running") steps.append(step("running", "En cours…", ""));
  return el("div", {}, summary, steps);
}

$("#trace-refresh").addEventListener("click", () => loadTraces());
$("#trace-back").addEventListener("click", () => showAutoList());

// ----- Confirmation d'identité

let afterConfirmation = null;

function askConfirmation(then) {
  afterConfirmation = then;
  const form = $("#confirm-form");
  form.reset();
  $("#confirm-error").hidden = true;
  $("#confirm").showModal();
  form.password.focus();
}

$("#confirm-cancel").addEventListener("click", () => {
  afterConfirmation = null;
  $("#confirm").close();
});

$("#confirm-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const button = form.querySelector("button[type=submit]");
  button.disabled = true;
  try {
    const code = form.code.value.trim();
    const { status } = await api("POST", "/api/auth/confirm", {
      password: form.password.value,
      ...(code && { code }),
    });
    form.password.value = "";
    form.code.value = "";
    if (status !== 200) {
      const error = $("#confirm-error");
      error.textContent = status === 429 ? "Trop de tentatives. Réessaie dans 15 minutes." : "Mot de passe ou code incorrect.";
      error.hidden = false;
      return;
    }
    $("#confirm").close();
    const then = afterConfirmation;
    afterConfirmation = null;
    then?.();
  } finally {
    button.disabled = false;
  }
});

/** Suggestions d'appareils pour les champs « Appareil(s) ». */
function refreshEntityList() {
  const list = $("#entity-list");
  const sorted = [...entities.values()].sort((a, b) => a.entity_id.localeCompare(b.entity_id));
  list.replaceChildren(...sorted.map((state) => el("option", { value: state.entity_id, label: nameOf(state) })));
}
