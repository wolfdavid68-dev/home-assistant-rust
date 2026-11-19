// ---------------------------------------------------------------- Automatisations toutes prêtes
// Étape 85 : départ, retour, nuit, matin, vacances. En haut de la liste des
// automatisations : on coche, on règle, et le serveur fabrique l'automatisation
// d'après les appareils de la maison (pret_depart…). Elle reste ensuite une
// automatisation ordinaire, dans la liste, avec son historique.

const PRESET_ICONS = {
  depart: ICONS.door,
  retour: ICONS.home,
  nuit: ICONS.moon,
  matin: ICONS.illuminance,
  vacances: ["M4 8h16v12H4z", "M9 8V5h6v3", "M4 13h16"],
  lessive: ["M5 3h14v18H5z", "M5 7h14", "M12 10a4 4 0 1 0 0 8 4 4 0 1 0 0-8z", "M8 5h.01"],
};

const presets = { list: [], house: null, editable: false, open: new Set() };

async function renderPresets() {
  const box = $("#auto-presets");
  const { status, data } = await api("GET", "/api/automations/presets");
  if (status !== 200) {
    box.hidden = true;
    return;
  }
  presets.list = data.presets ?? [];
  presets.house = data.house ?? {};
  presets.editable = data.editable === true;
  box.hidden = false;
  $("#auto-preset-list").replaceChildren(...presets.list.map(presetCard));
}

/** « Éteindre les lumières · Chauffage à 17 °C · … » : ce qu'elle fait. */
function presetSummary(preset) {
  const o = preset.options;
  const h = presets.house;
  const parts = [];
  if (preset.name === "depart") parts.push(`après ${o.delai_minutes} min`);
  if (preset.name === "vacances") parts.push(`après ${o.apres_heures} h`);
  if (preset.name === "nuit") parts.push(`à ${o.heure}`);
  if (preset.name === "matin") parts.push(`à ${o.heure} (week-end ${o.heure_weekend})`);
  if (o.lumieres && h.lights?.length) parts.push(preset.name === "nuit" && !o.chambres ? "éteindre les lumières, sauf les chambres" : "éteindre les lumières");
  if (o.volets && h.covers) {
    if (preset.name === "matin") parts.push(o.chambres ? "ouvrir les volets" : "ouvrir les volets, sauf les chambres");
    else parts.push("fermer les volets");
  }
  if (o.chauffer && h.climates) parts.push(`chauffage à ${String(o.chauffage).replace(".", ",")} °C`);
  if (preset.name === "retour" && o.lumiere_entree) {
    const light = h.lights?.find((l) => l.entity_id === o.lumiere_entree);
    parts.push(`allumer ${light?.name ?? o.lumiere_entree} s'il fait nuit`);
  }
  if (o.ouvertures && h.openings) parts.push("prévenir si une porte ou une fenêtre est ouverte");
  if (preset.name === "vacances" && o.simulation && h.simulation) parts.push("simulation de présence");
  if (preset.name === "lessive") {
    const machines = (h.appliances ?? []).map((a) => a.name).join(", ");
    if (machines) parts.push(machines);
    if (o.notifier) parts.push("message sur le téléphone");
    if (o.annonce && h.announce) parts.push("annonce sur les enceintes");
  }
  return parts.join(" · ");
}

function presetCard(preset) {
  const card = el("li", { class: `bg-card-2 [border-radius:18px] [padding:0.8rem_0.9rem] grid gap-[0.45rem] [align-content:start] [border:1px_solid_transparent] [&.is-on]:border-accent [&.is-unavailable]:[opacity:0.6] preset-card${preset.enabled ? " is-on" : ""}${preset.unavailable ? " is-unavailable" : ""}` });
  const toggle = el("input", { type: "checkbox", "aria-label": `${preset.title} : active` });
  toggle.checked = preset.enabled;
  toggle.disabled = !presets.editable || Boolean(preset.unavailable);
  toggle.addEventListener("change", () => savePreset(preset, toggle.checked, preset.options));
  card.append(
    el(
      "div",
      { class: "preset-head grid [grid-template-columns:auto_minmax(0,_1fr)_auto] gap-[0.6rem] items-center" },
      el("span", { class: "preset-icon grid [place-items:center] w-[36px] h-[36px] [border-radius:12px] bg-card text-accent [:not(:where(.o032000))&_svg]:w-[20px] [:not(:where(.o032001))&_svg]:h-[20px]" }, svgIcon(PRESET_ICONS[preset.name] ?? ICONS.sparkle)),
      el("div", { class: "preset-title grid [:not(:where(.o032200))&_small]:text-muted [:not(:where(.o032201))&_small]:text-[0.8rem]" }, el("strong", { text: preset.title }), el("small", { text: preset.when })),
      el("label", { class: "preset-switch relative inline-block w-[44px] h-[26px] cursor-pointer [:not(:where(.o032500))&_input]:absolute [:not(:where(.o032501))&_input]:[inset:0] [&_input]:[opacity:0] [&_input]:m-0 [&_input]:[cursor:inherit] [:not(:where(.o032600))&_span]:absolute [:not(:where(.o032601))&_span]:[inset:0] [:not(:where(.o032602))&_span]:[border-radius:999px] [:not(:where(.o032603))&_span]:bg-line [:not(:where(.o032604))&_span]:[transition:background_0.2s] [&_span]:pointer-events-none [&_span::after]:[content:''] [&_span::after]:absolute [:not(:where(.o032702))&_span::after]:[top:3px] [:not(:where(.o032703))&_span::after]:[left:3px] [&_span::after]:w-[20px] [&_span::after]:h-[20px] [&_span::after]:[border-radius:50%] [:not(:where(.o032707))&_span::after]:[background:#fff] [&_span::after]:[box-shadow:0_1px_3px_rgb(0_0_0_/_0.25)] [:not(:where(.o032709))&_span::after]:[transition:transform_0.2s] [:not(:where(.o032800))&_input:checked+span]:bg-accent [&_input:checked+span::after]:[transform:translateX(18px)] [&_input:focus-visible+span]:[outline:2px_solid_var(--accent)] [&_input:focus-visible+span]:[outline-offset:2px] [&_input:disabled+span]:[opacity:0.5] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o034000))&_span]:[transition:none] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o034000))&_span::after]:[transition:none]" }, toggle, el("span", { "aria-hidden": "true" })),
    ),
  );
  if (preset.unavailable) {
    card.append(el("p", { class: "preset-summary m-0 text-text-2 text-[0.85rem] [overflow-wrap:anywhere]", text: `Pas possible ici : ${preset.unavailable}.` }));
    return card;
  }
  card.append(el("p", { class: "preset-summary m-0 text-text-2 text-[0.85rem] [overflow-wrap:anywhere]", text: presetSummary(preset) }));
  if (!presets.editable) return card;
  const details = el("details", { class: "preset-details [:not(:where(.o033200))&_summary]:cursor-pointer [:not(:where(.o033201))&_summary]:text-accent [:not(:where(.o033202))&_summary]:text-[0.88rem] [:not(:where(.o033203))&_summary]:font-semibold" }, el("summary", { text: "Régler" }));
  details.open = presets.open.has(preset.name);
  details.addEventListener("toggle", () => (details.open ? presets.open.add(preset.name) : presets.open.delete(preset.name)));
  const draft = structuredClone(preset.options);
  const form = el("div", { class: "preset-form grid gap-2 mt-[0.6rem]" }, ...presetFields(preset.name, draft));
  const save = el("button", { type: "button", class: "primary small", text: preset.enabled ? "Enregistrer" : "Enregistrer et activer" });
  save.addEventListener("click", () => savePreset(preset, true, draft));
  details.append(form, el("div", { class: "preset-buttons flex justify-end mt-[0.6rem]" }, save));
  card.append(details);
  return card;
}

/** Une case à cocher liée à `draft[key]`. */
function presetCheck(draft, key, text) {
  const input = el("input", { type: "checkbox" });
  input.checked = draft[key] === true;
  input.addEventListener("change", () => (draft[key] = input.checked));
  return el("label", { class: "check [.preset-form_&]:text-[0.9rem] [label&]:flex [label&]:items-center [label&]:gap-[0.45rem] [label&]:text-text" }, input, ` ${text}`);
}

function presetNumber(draft, key, text, { min, max, step = 1, unit }) {
  const input = el("input", { type: "number", min: String(min), max: String(max), step: String(step), inputmode: "decimal" });
  input.value = draft[key] ?? "";
  input.addEventListener("input", () => {
    const value = Number(input.value);
    if (input.value !== "" && Number.isFinite(value)) draft[key] = value;
  });
  return el("label", { class: "preset-field flex flex-wrap items-center justify-between gap-2 text-[0.9rem] [:not(:where(.o033600))&_select]:max-w-[60%]" }, el("span", { text }), el("span", { class: "preset-input inline-flex items-center gap-[0.35rem] [:not(:where(.o033800))&_input]:w-22" }, input, el("span", { text: unit })));
}

function presetTime(draft, key, text) {
  const input = el("input", { type: "time", step: "60" });
  input.value = draft[key] ?? "";
  input.addEventListener("input", () => {
    if (/^\d\d:\d\d$/.test(input.value)) draft[key] = input.value;
  });
  return el("label", { class: "preset-field flex flex-wrap items-center justify-between gap-2 text-[0.9rem] [:not(:where(.o033600))&_select]:max-w-[60%]" }, el("span", { text }), input);
}

/** Le chauffage : une case, et la température. */
function presetHeating(draft, text) {
  if (!presets.house.climates) return [];
  return [presetCheck(draft, "chauffer", text), presetNumber(draft, "chauffage", "Consigne", { min: 5, max: 30, step: 0.5, unit: "°C" })];
}

function presetFields(name, draft) {
  const h = presets.house;
  const count = (n, one, many) => (n === 1 ? `1 ${one}` : `${n} ${many}`);
  const lights = h.lights?.length ?? 0;
  const fields = [];
  const openings = h.openings ? [presetCheck(draft, "ouvertures", `Prévenir si une porte ou une fenêtre est restée ouverte (${count(h.openings, "suivie", "suivies")})`)] : [];
  if (name === "depart") {
    fields.push(presetNumber(draft, "delai_minutes", "Quand la maison est vide depuis", { min: 1, max: 240, unit: "min" }));
    if (lights) fields.push(presetCheck(draft, "lumieres", `Éteindre les lumières (${count(lights, "lumière", "lumières")})`));
    if (h.covers) fields.push(presetCheck(draft, "volets", `Fermer les volets (${count(h.covers, "volet", "volets")})`));
    fields.push(...presetHeating(draft, "Baisser le chauffage"), ...openings);
  } else if (name === "retour") {
    fields.push(...presetHeating(draft, "Remettre le chauffage"));
    if (lights) {
      const select = el("select", {}, el("option", { value: "", text: "Aucune" }), ...h.lights.map((l) => el("option", { value: l.entity_id, text: l.name })));
      select.value = draft.lumiere_entree ?? "";
      select.addEventListener("change", () => (draft.lumiere_entree = select.value));
      fields.push(el("label", { class: "preset-field flex flex-wrap items-center justify-between gap-2 text-[0.9rem] [:not(:where(.o033600))&_select]:max-w-[60%]" }, el("span", { text: h.sun ? "Allumer s'il fait nuit" : "Allumer" }), select));
    }
  } else if (name === "nuit") {
    fields.push(presetTime(draft, "heure", "Chaque soir à"));
    if (lights) fields.push(presetCheck(draft, "lumieres", "Éteindre les lumières oubliées"));
    if (lights && h.bedroom_lights) fields.push(presetCheck(draft, "chambres", `Aussi dans les chambres (${count(h.bedroom_lights, "lumière", "lumières")})`));
    if (h.covers) fields.push(presetCheck(draft, "volets", `Fermer les volets (${count(h.covers, "volet", "volets")})`));
    fields.push(...presetHeating(draft, "Changer le chauffage"), ...openings);
  } else if (name === "matin") {
    fields.push(presetTime(draft, "heure", "En semaine à"), presetTime(draft, "heure_weekend", "Le week-end à"));
    if (h.covers) fields.push(presetCheck(draft, "volets", "Ouvrir les volets"));
    if (h.covers && h.bedroom_covers) fields.push(presetCheck(draft, "chambres", `Aussi ceux des chambres (${count(h.bedroom_covers, "volet", "volets")})`));
    fields.push(...presetHeating(draft, "Remettre le chauffage"));
  } else if (name === "lessive") {
    fields.push(presetCheck(draft, "notifier", "Un message sur le téléphone (« Lave-linge a fini, en 92 minutes »)"));
    if (h.announce) fields.push(presetCheck(draft, "annonce", "L'annoncer sur les enceintes"));
  } else if (name === "vacances") {
    fields.push(presetNumber(draft, "apres_heures", "Quand la maison est vide depuis", { min: 1, max: 24, unit: "h" }));
    fields.push(...presetHeating(draft, "Mettre le chauffage en hors-gel"));
    if (h.simulation) fields.push(presetCheck(draft, "simulation", "Armer la simulation de présence (désarmée au retour)"));
  }
  return fields;
}

async function savePreset(preset, enabled, options) {
  const { status, data } = await api("PUT", `/api/automations/presets/${encodeURIComponent(preset.name)}`, { enabled, options });
  if (status === 403 && data.confirm) {
    // Annulé : la case reprend sa place.
    renderPresets();
    return askConfirmation(() => savePreset(preset, enabled, options));
  }
  if (status !== 200) {
    await renderPresets();
    return showPresetNotice(data.error ?? "Enregistrement impossible.", true);
  }
  presets.open.delete(preset.name);
  await showAutoList(enabled ? `« ${preset.title} » est prête, et active.` : `« ${preset.title} » est désactivée (gardée).`);
}

function showPresetNotice(text, error = false) {
  const notice = $("#auto-notice");
  notice.textContent = text;
  notice.classList.toggle("error", error);
  notice.hidden = false;
}
