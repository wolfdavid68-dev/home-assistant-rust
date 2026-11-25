// Accueil v4 : la page Énergie (maquette « Hub maison »).
//
// Les compteurs comptent depuis la mise en service. La pompe à chaleur
// (Atlantic, par la box Somfy) en a deux : le chauffage et l'eau chaude ;
// une barre partagée les montre, avec une bulle au survol. Le lave-linge
// (SmartThings ou prise mesurée) se compare à elle. Plus bas, les autres
// appareils mesurés et le compteur général (Linky), s'il est relié.
//
// La page elle-même (fond, barre du haut, colonnes) est celle de
// hub-v4-pieces.js : ce fichier n'en fournit que le contenu.
"use strict";

const HUB_V4_PAC = {
  heating: "sensor.pac_consommation_chauffage",
  water: "sensor.pac_consommation_eau_chaude",
  total: "sensor.pac_consommation_totale",
};
// Les couleurs de la maquette : chauffage orangé, eau chaude bleu canard.
const HUB_V4_PAC_TONES = { heating: "bg-[#f4511e]", water: "bg-[#0097a7]" };
// Tempo : les couleurs du jour EDF, les mêmes que l'incrustation Tempo
// (pas de jeton : ce sont les couleurs d'EDF, pareilles de jour et de nuit).
const HUB_V4_TEMPO_TONES = { Bleu: "blue", Blanc: "white", Rouge: "red" };
const hubV4Kwh0 = new Intl.NumberFormat("fr", { maximumFractionDigits: 0 });
const hubV4Kwh1 = new Intl.NumberFormat("fr", { maximumFractionDigits: 1 });

// ------------------------------------------------------- Calculs (testés)

/** Un compteur en kWh : un nombre positif, ou null. */
function hubV4EnergyValue(state) {
  if (!state) return null;
  const value = Number(state.state);
  return state.state !== "" && Number.isFinite(value) && value >= 0 ? value : null;
}

/** « 3 832 », « 4,6 » (une décimale sous 10 kWh). */
function hubV4KwhText(kwh) {
  return (kwh < 10 ? hubV4Kwh1 : hubV4Kwh0).format(kwh);
}

/**
 * Les parts d'un total : chacune avec son pourcentage entier, et les
 * pourcentages font 100 (plus forts restes). `parts` : [{ key, kwh }].
 */
function hubV4EnergyShares(parts) {
  const total = parts.reduce((sum, part) => sum + part.kwh, 0);
  if (total <= 0) return { total: 0, parts: parts.map((part) => ({ ...part, pct: 0 })) };
  const raw = parts.map((part) => (part.kwh / total) * 100);
  const pct = raw.map(Math.floor);
  let left = 100 - pct.reduce((a, b) => a + b, 0);
  const order = raw.map((value, index) => [value - Math.floor(value), index]).sort((a, b) => b[0] - a[0]);
  for (const [, index] of order) {
    if (left <= 0) break;
    pct[index] += 1;
    left -= 1;
  }
  return { total, parts: parts.map((part, index) => ({ ...part, pct: pct[index] })) };
}

/** Un appareil face à la pompe à chaleur : « À peu près 8 % de ce que consomme la pompe à chaleur. » */
function hubV4EnergyCompare(kwh, pac) {
  if (!(pac > 0)) return "";
  if (kwh >= pac) return "Plus que la pompe à chaleur.";
  const pct = Math.round((kwh / pac) * 100);
  if (pct < 1) return "Moins de 1 % de ce que consomme la pompe à chaleur.";
  return `À peu près ${pct} % de ce que consomme la pompe à chaleur.`;
}

/**
 * Les autres compteurs d'énergie (kWh) : ni la pompe à chaleur, ni le Linky,
 * ni ceux déjà montrés (`taken`). Du plus gros au plus petit.
 */
function hubV4EnergyMeters(states, taken) {
  return states
    .filter((s) => s.entity_id.startsWith("sensor.") && s.attributes.unit_of_measurement === "kWh")
    .filter((s) => s.attributes.device_class === "energy" || s.attributes.state_class === "total_increasing")
    .filter((s) => !s.entity_id.startsWith("sensor.pac_consommation") && !s.entity_id.startsWith("sensor.linky_"))
    .filter((s) => !taken.includes(s.entity_id) && hubV4EnergyValue(s) !== null)
    .sort((a, b) => hubV4EnergyValue(b) - hubV4EnergyValue(a));
}

/** La phrase sous le titre. */
function hubV4EnergyLead(pac, others) {
  if (!(pac > 0)) return others > 0 ? "Ce que consomment les appareils mesurés." : "Aucun compteur d'énergie pour l'instant.";
  return pac > others
    ? "La pompe à chaleur fait presque tout : elle chauffe la maison et l'eau."
    : "La pompe à chaleur chauffe la maison et l'eau.";
}

/** Une journée Tempo : sa teinte (blue, white, red ou unknown) et son mot. */
function hubV4EnergyTempo(state) {
  if (!state) return { tone: "unknown", text: "—" };
  if (Object.hasOwn(HUB_V4_TEMPO_TONES, state.state)) return { tone: HUB_V4_TEMPO_TONES[state.state], text: state.state };
  if (state.state === "unavailable") return { tone: "unknown", text: "Indisponible" };
  return { tone: "unknown", text: "À venir" };
}

// --------------------------------------------------------- Les morceaux

/** Les compteurs de la pompe à chaleur : les deux parts si elles y sont, sinon le total seul. */
function hubV4PacReading() {
  const heating = hubV4EnergyValue(entities.get(HUB_V4_PAC.heating));
  const water = hubV4EnergyValue(entities.get(HUB_V4_PAC.water));
  if (heating !== null && water !== null) {
    return hubV4EnergyShares([
      { key: "heating", name: "Chauffage", kwh: heating },
      { key: "water", name: "Eau chaude", kwh: water },
    ]);
  }
  const total = hubV4EnergyValue(entities.get(HUB_V4_PAC.total));
  return total === null ? null : { total, parts: [] };
}

/** Les machines (lave-linge…) qui ont un compteur : `sensor.lave_linge` → `sensor.lave_linge_energie`. */
function hubV4EnergyMachines() {
  return [...entities.values()]
    .filter((s) => domainOf(s) === "sensor" && typeof s.attributes.appareil === "string")
    .map((s) => ({ machine: s, meter: entities.get(`${s.entity_id}_energie`) }))
    .filter((m) => hubV4EnergyValue(m.meter) !== null)
    .sort((a, b) => nameOf(a.machine).localeCompare(nameOf(b.machine), "fr"));
}

/** La ligne sous le titre d'une tuile : « Chauffage et eau chaude », « Relevé par SmartThings ». */
function hubV4EnergySub(text = "") {
  return el("p", { class: `m-0 -mt-2.5 text-petit ${HUB_V4_MUTE}`, text });
}

/** Un grand nombre en chiffres fins : « 3 832 kWh ». */
function hubV4EnergyBig(size = "text-affiche-l", unit = "kWh") {
  const value = el("b", { class: `font-chiffres ${size} font-extralight leading-none tracking-[-0.03em]` });
  const unitNode = el("span", { class: `text-corps ${HUB_V4_MUTE}`, text: unit });
  return { node: el("p", { class: "m-0 flex items-baseline gap-1.5" }, value, unitNode), value, unit: unitNode };
}

/** La pompe à chaleur : le total, la barre partagée (avec sa bulle), la légende. */
function hubV4PacTile() {
  const tile = hubV4Tile(ICONS.flame, "heat", "Pompe à chaleur");
  const per = hubV4EnergySub();
  const big = hubV4EnergyBig();
  const bar = el("div", { class: "flex gap-0.5 h-3.5", role: "img" });
  const legend = el("ul", { class: "m-0 p-0 list-none grid grid-cols-2 gap-2" });
  const sum = el("p", { class: `m-0 text-legende ${HUB_V4_MUTE}` });
  tile.node.append(per, big.node, bar, legend, sum);
  const refresh = () => {
    const pac = hubV4PacReading();
    tile.node.hidden = pac === null;
    if (!pac) return;
    setText(big.value, hubV4KwhText(pac.total));
    setText(per, pac.parts.length ? "Chauffage et eau chaude" : "Consommation totale");
    bar.hidden = legend.hidden = sum.hidden = pac.parts.length === 0;
    const mark = pac.parts.map((p) => `${p.key}:${p.kwh}`).join("|");
    if (bar.dataset.mark === mark) return;
    bar.dataset.mark = mark;
    const said = pac.parts.map((p) => `${p.name} ${p.pct} %`).join(", ");
    setAttr(bar, "aria-label", said);
    bar.replaceChildren(...pac.parts.filter((p) => p.kwh > 0).map((part, index, list) => {
      const words = `${part.name} · ${hubV4KwhText(part.kwh)} kWh · ${part.pct} %`;
      // La bulle : au survol ou au focus (clavier), au-dessus de sa part.
      // Calée sur le bord de la tuile pour la première et la dernière part,
      // pour ne pas déborder de la colonne.
      const side = index === 0 ? "left-0" : index === list.length - 1 ? "right-0" : "left-1/2 -translate-x-1/2";
      const tip = el("span", {
        role: "tooltip",
        class: `pointer-events-none absolute bottom-[calc(100%+10px)] ${side} z-10 whitespace-nowrap `
          + "px-3 py-1.5 rounded-[10px] bg-v-ink text-v-inverse text-legende font-medium shadow-lg "
          + "invisible opacity-0 translate-y-1 transition-all duration-150 group-hover:visible group-hover:opacity-100 group-hover:translate-y-0 "
          + "group-focus-visible:visible group-focus-visible:opacity-100 group-focus-visible:translate-y-0 motion-reduce:transition-none",
      }, el("b", { text: part.name }), ` · ${hubV4KwhText(part.kwh)} kWh · ${part.pct} %`);
      const segment = el("span", {
        tabindex: "0", "aria-label": words,
        class: `group relative grow-(--g) basis-0 min-w-1.5 ${HUB_V4_PAC_TONES[part.key]} cursor-default `
          + `${index === 0 ? "rounded-l-[6px]" : ""} ${index === list.length - 1 ? "rounded-r-[6px]" : ""} ${HUB_V4_FOCUS}`,
      }, tip);
      segment.style.setProperty("--g", String(part.kwh));
      return segment;
    }));
    // La légende : une carte par part, comme la maquette.
    legend.replaceChildren(...pac.parts.map((part) => el("li", { class: "flex gap-2.5 min-w-0 p-3 rounded-[16px] bg-voile/6" },
      el("span", { class: `mt-1 size-2.5 shrink-0 rounded-[3px] ${HUB_V4_PAC_TONES[part.key]}`, "aria-hidden": "true" }),
      el("p", { class: "m-0 flex flex-col gap-0.5 min-w-0 text-corps" },
        el("span", { class: "font-semibold", text: part.name }),
        el("span", { class: "font-chiffres text-grand font-light leading-tight whitespace-nowrap", text: `${hubV4KwhText(part.kwh)} kWh` }),
        el("span", { class: `text-legende ${HUB_V4_MUTE}`, text: `${part.pct} % du total` })))));
    setText(sum, `Les deux parts font le total : ${pac.parts.map((p) => hubV4KwhText(p.kwh)).join(" + ")} = ${hubV4KwhText(pac.total)} kWh.`);
  };
  return { node: tile.node, refresh };
}

/** En ce moment : le chauffage (le mode de chaque zone) et l'eau chaude. */
function hubV4NowTile() {
  const tile = hubV4Tile(ICONS.timer, "amber", "En ce moment");
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-3" });
  tile.node.append(list);
  const row = (icon, title, sub, value) => el("li", { class: "flex items-center gap-3" },
    el("span", { class: `${HUB_V4_MUTE} [&_svg]:size-5`, "aria-hidden": "true" }, svgIcon(icon)),
    el("p", { class: "m-0 flex-1 min-w-0 flex flex-col text-corps font-semibold" }, title,
      el("span", { class: `text-legende font-normal truncate ${HUB_V4_MUTE}`, text: sub })),
    // Une mesure en chiffres fins ; un mot (« Chauffage ») plus discret, son
    // point orangé (le texte reste à l'encre : l'orangé du jour n'atteint pas AA).
    /\d/.test(value)
      ? el("span", { class: "font-chiffres text-titre-l font-extralight leading-none whitespace-nowrap", text: value })
      : el("span", { class: "flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-v-heat/14 text-corps font-semibold whitespace-nowrap" },
        el("span", { class: "size-1.5 shrink-0 rounded-full bg-v-heat", "aria-hidden": "true" }), value));
  const refresh = () => {
    const rows = [];
    // Une ligne par zone : son mode (Confort, Éco…) et la température mesurée.
    for (const zone of hubV4HeatZones()) {
      const state = entities.get(zone.id);
      if (!state) continue;
      const current = state.attributes.current_temperature;
      rows.push(["flame", hubV4ShortName(nameOf(state), "Chauffage"), `Chauffage · ${heatMode(state)}`,
        finite(current) ? `${hubV4Kwh1.format(current)}°` : "—"]);
    }
    const water = entities.get(HUB_V3_WATER);
    if (water) {
      const current = water.attributes.current_temperature;
      rows.push(["humidity", "Eau chaude", finite(current) ? "dans le ballon" : heatMode(water), finite(current) ? `${hubV4Kwh1.format(current)}°` : "—"]);
    }
    const pump = hubV3Pump();
    if (pump) rows.push(["bolt", "Pompe à chaleur", "en ce moment", pump.state.charAt(0).toUpperCase() + pump.state.slice(1)]);
    tile.node.hidden = rows.length === 0;
    const mark = rows.map((r) => r.join(":")).join("|");
    if (list.dataset.mark === mark) return;
    list.dataset.mark = mark;
    const icons = { flame: ICONS.flame, humidity: ICONS.humidity, bolt: HUB_ICONS.bolt };
    list.replaceChildren(...rows.map(([icon, title, sub, value]) => row(icons[icon], title, sub, value)));
  };
  return { node: tile.node, refresh };
}

/** Une machine (le lave-linge, relevé par SmartThings) face à la pompe à chaleur. */
function hubV4MachineEnergyTile(machineId) {
  const tile = hubV4Tile(ICONS.washer, "violet", "");
  const big = hubV4EnergyBig("text-affiche");
  const compare = el("div", { class: "flex flex-col gap-2.5", role: "img" });
  const note = el("p", { class: `m-0 text-petit text-pretty ${HUB_V4_MUTE}` });
  tile.node.append(hubV4EnergySub("Relevé par SmartThings"), big.node, compare, note);
  const bar = (name, kwh, tone) => {
    const fill = el("i", { class: `block h-full rounded-full w-(--w) min-w-1.5 ${tone}` });
    fill.style.setProperty("--w", `${kwh.pct}%`);
    return el("div", { class: "flex flex-col gap-1.5" },
      el("p", { class: `m-0 flex justify-between gap-3 text-petit ${HUB_V4_MUTE}` }, el("span", { text: name }),
        el("span", { class: "font-chiffres whitespace-nowrap", text: `${hubV4KwhText(kwh.value)} kWh` })),
      el("span", { class: "block h-1.5 rounded-full bg-voile/8" }, fill));
  };
  const refresh = () => {
    const machine = entities.get(machineId);
    const kwh = hubV4EnergyValue(entities.get(`${machineId}_energie`));
    tile.node.hidden = !machine || kwh === null;
    if (tile.node.hidden) return;
    setText(tile.titleNode, nameOf(machine));
    setText(big.value, hubV4KwhText(kwh));
    const pac = hubV4PacReading()?.total ?? 0;
    compare.hidden = !(pac > 0);
    setText(note, hubV4EnergyCompare(kwh, pac) || "Depuis la mise en service.");
    const mark = `${pac}:${kwh}`;
    if (compare.dataset.mark === mark || compare.hidden) return;
    compare.dataset.mark = mark;
    const most = Math.max(pac, kwh);
    setAttr(compare, "aria-label", `${nameOf(machine)} : ${hubV4EnergyCompare(kwh, pac)}`);
    compare.replaceChildren(
      bar("Pompe à chaleur", { value: pac, pct: (pac / most) * 100 }, HUB_V4_PAC_TONES.heating),
      bar(nameOf(machine), { value: kwh, pct: (kwh / most) * 100 }, "bg-v-ink/70"));
  };
  return { node: tile.node, refresh };
}

/** Tempo : la couleur d'aujourd'hui et de demain, en bleu, blanc ou rouge. */
function hubV4TempoTile() {
  const tile = hubV4Tile(HUB_ICONS.bolt, "sky", "Tempo");
  const day = (label) => {
    const dot = el("span", {
      "aria-hidden": "true",
      class: "size-3.5 shrink-0 rounded-full border-2 border-dashed border-voile/40 "
        + "data-[tone=blue]:border-0 data-[tone=blue]:bg-[#5b8cff] "
        + "data-[tone=white]:border-0 data-[tone=white]:bg-white data-[tone=white]:ring-1 data-[tone=white]:ring-voile/40 "
        + "data-[tone=red]:border-0 data-[tone=red]:bg-[#ff6b6b]",
    });
    const word = el("b", { class: "text-grand font-semibold" });
    // La carte du jour prend un reflet de sa couleur.
    const node = el("div", {
      class: "flex flex-col gap-1.5 p-3 rounded-[16px] bg-voile/6 ring-1 ring-inset ring-transparent "
        + "data-[tone=blue]:bg-[#5b8cff]/16 data-[tone=blue]:ring-[#5b8cff]/45 "
        + "data-[tone=white]:bg-voile/10 data-[tone=white]:ring-voile/30 "
        + "data-[tone=red]:bg-[#ff6b6b]/16 data-[tone=red]:ring-[#ff6b6b]/50",
    }, el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: label }), el("span", { class: "flex items-center gap-2" }, dot, word));
    return { node, dot, word };
  };
  const today = day("Aujourd'hui");
  const tomorrow = day("Demain");
  const note = el("p", { class: `m-0 text-petit ${HUB_V4_MUTE}` });
  tile.node.append(hubV4EnergySub("La couleur du jour EDF"),
    el("div", { class: "grid grid-cols-2 gap-2" }, today.node, tomorrow.node), note);
  const paint = (part, state) => {
    const { tone, text } = hubV4EnergyTempo(state);
    part.node.dataset.tone = part.dot.dataset.tone = tone;
    setText(part.word, text);
  };
  const refresh = () => {
    const now = entities.get(TEMPO_TODAY);
    tile.node.hidden = !now;
    if (!now) return;
    const next = entities.get(TEMPO_TOMORROW);
    paint(today, now);
    paint(tomorrow, next);
    setText(note, hubV4EnergyTempo(next).tone === "unknown" ? "La couleur de demain arrive vers 11 h." : "Un jour Tempo va de 6 h à 6 h.");
  };
  return { node: tile.node, refresh };
}

/** Les autres appareils mesurés, ou une place qui les attend. */
function hubV4OthersTile(taken) {
  const tile = hubV4Tile(ICONS.plug, "violet", "Autres appareils");
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-2" });
  const empty = el("p", {
    class: `m-0 px-4 py-4 rounded-[16px] border border-dashed border-voile/18 text-center text-petit ${HUB_V4_MUTE}`,
    text: "Pas encore mesurés",
  });
  tile.node.append(list, empty);
  const refresh = () => {
    const meters = hubV4EnergyMeters([...entities.values()], taken()).slice(0, 6);
    empty.hidden = meters.length > 0;
    list.hidden = meters.length === 0;
    const mark = meters.map((s) => `${s.entity_id}:${s.state}`).join("|");
    if (list.dataset.mark === mark) return;
    list.dataset.mark = mark;
    list.replaceChildren(...meters.map((s) => el("li", { class: "flex items-baseline justify-between gap-3 text-corps" },
      el("span", { class: "truncate", text: nameOf(s) }),
      el("span", { class: `font-chiffres whitespace-nowrap ${HUB_V4_MUTE}`, text: `${hubV4KwhText(hubV4EnergyValue(s))} kWh` }))));
  };
  return { node: tile.node, refresh };
}

/** Le compteur général : la puissance du moment et le jour, ou comment le relier. */
function hubV4MeterTile() {
  const tile = hubV4Tile(HUB_ICONS.bolt, "amber", "Compteur général");
  // Relié : la puissance en chiffres fins ; sinon une phrase en serif.
  const big = hubV4EnergyBig("text-affiche", "VA en ce moment");
  const title = el("p", { class: "m-0 font-recit text-titre-l font-normal leading-tight" });
  const text = el("p", { class: `m-0 text-petit text-pretty ${HUB_V4_MUTE}` });
  const button = el("button", { type: "button", class: `${HUB_V4_BTN} self-start`, text: "Comment le relier" });
  button.addEventListener("click", () => {
    if (typeof openEnergy === "function") openEnergy();
  });
  tile.node.append(big.node, title, text, button);
  let today = null;
  // Le jour en cours : une fois à l'ouverture (le bilan est calculé à l'heure).
  const load = async () => {
    if (typeof api !== "function") return;
    const { status, data } = await api("GET", "/api/energy?days=1").catch(() => ({ status: 0, data: {} }));
    if (status === 200 && (data.sources ?? []).length > 0) today = data.today?.consumed_kwh ?? null;
    refresh();
  };
  const refresh = () => {
    const linky = [...entities.keys()].some((id) => id.startsWith("sensor.linky_"));
    const power = hubV4EnergyValue(entities.get("sensor.linky_puissance"));
    button.hidden = linky;
    big.node.hidden = !linky || power === null;
    title.hidden = !big.node.hidden;
    if (!linky) {
      setText(title, "Pas encore relié");
      setText(text, "Une fois le module de téléinformation branché, la consommation de toute la maison s'affichera ici.");
      return;
    }
    if (power !== null) setText(big.value, hubV4Kwh0.format(power));
    else setText(title, "Relié");
    setText(text, today !== null ? `${hubV4KwhText(today)} kWh depuis minuit (heure en cours non comptée).` : "La consommation du jour arrive à la fin de l'heure.");
  };
  load();
  return { node: tile.node, refresh };
}

/** Le contenu de la page Énergie, pour hubV4Page(HUB_V4_ENERGY). */
function hubV4EnergyBody() {
  const paints = [];
  // L'en-tête de la page Pièce : le fil, « Énergie » en serif, la phrase.
  const lead = el("p", { class: "m-0 text-corps text-pretty text-v-ink2" });
  const left = [hubV4PageHead("Depuis la mise en service", "Énergie", lead)];
  const pac = hubV4PacTile();
  const now = hubV4NowTile();
  left.push(pac.node, now.node);
  paints.push(pac.refresh, now.refresh);

  const right = [];
  const machines = hubV4EnergyMachines();
  for (const { machine } of machines) {
    const tile = hubV4MachineEnergyTile(machine.entity_id);
    right.push(tile.node);
    paints.push(tile.refresh);
  }
  const taken = () => hubV4EnergyMachines().map((m) => m.meter.entity_id);
  const tempo = hubV4TempoTile();
  const meter = hubV4MeterTile();
  const others = hubV4OthersTile(taken);
  right.push(tempo.node, meter.node, others.node);
  paints.push(tempo.refresh, meter.refresh, others.refresh);

  // Sous le dessin, la phrase de la maquette.
  const caption = el("p", { class: "m-0 px-5 py-2.5 rounded-full bg-scrim/60 backdrop-blur-[14px] text-moyen text-center" },
    "Toute la chaleur vient de la ", el("b", { class: "text-v-amber", text: "pompe à chaleur" }), ".");
  paints.push(() => {
    const total = hubV4PacReading()?.total ?? 0;
    const rest = hubV4EnergyMachines().reduce((sum, m) => sum + hubV4EnergyValue(m.meter), 0)
      + hubV4EnergyMeters([...entities.values()], taken()).reduce((sum, s) => sum + hubV4EnergyValue(s), 0);
    setText(lead, hubV4EnergyLead(total, rest));
    caption.hidden = !(total > 0);
  });

  return { left, right, middle: [caption], refresh: () => paints.forEach((paint) => paint()) };
}

// En v4, le bouton Énergie de l'accueil ouvre cette page ; le bilan détaillé
// (graphique jour par jour) est à un toucher de là.
for (const id of ["#hub-energy", "#dock-energy"]) {
  document.querySelector?.(id)?.addEventListener("click", (event) => {
    if (typeof hubV4Page === "function" && hubV4Page(HUB_V4_ENERGY)) event.stopImmediatePropagation();
  }, { capture: true });
}
